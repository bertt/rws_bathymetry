/**
 * CompositeTerrainProvider
 *
 * CesiumJS supports only one terrain provider. This provider wraps a high priority
 * "overlay" provider (bathymetry, with holes where there is no survey data) and a
 * base provider (PDOK DTM). For every tile it samples both and builds a heightmap tile
 * (ellipsoidal heights): overlay height where it has data, base height elsewhere.
 * Tiles the overlay does not cover are returned unchanged from the base provider.
 */
class CompositeTerrainProvider {
  constructor(base, overlay, options = {}) {
    this.base = base;
    this.overlay = overlay;
    this.gridSize = options.gridSize || 65;
    this.minOverlayLevel = options.minOverlayLevel || 7; // coarse levels hold flat placeholder tiles
    this.fallbackHeight = options.fallbackHeight ?? 0; // ellipsoid height used where neither source has data
    this.errorEvent = new Cesium.Event();
    this.credit = base.credit;
    this.tilingScheme = base.tilingScheme;
    this.hasWaterMask = false;
    this.hasVertexNormals = false;
    this.availability = undefined;
  }

  getLevelMaximumGeometricError(level) {
    return this.base.getLevelMaximumGeometricError(level);
  }

  getTileDataAvailable(x, y, level) {
    if (this.overlayHas(x, y, level)) return true;
    return this.base.getTileDataAvailable(x, y, level);
  }

  loadTileDataAvailability(x, y, level) {
    return this.base.loadTileDataAvailability(x, y, level);
  }

  overlayHas(x, y, level) {
    return level >= this.minOverlayLevel && this.overlay.getTileDataAvailable(x, y, level) === true;
  }

  // Nearest available ancestor of the base provider (PDOK stops at level 15).
  async baseTile(x, y, level, request) {
    while (level > 0 && this.base.getTileDataAvailable(x, y, level) === false) {
      x >>= 1; y >>= 1; level--;
    }
    try {
      const data = await this.base.requestTileGeometry(x, y, level, request);
      return data ? { data, x, y, level } : undefined;
    } catch (e) {
      return undefined;
    }
  }

  async requestTileGeometry(x, y, level, request) {
    if (!this.overlayHas(x, y, level)) {
      return this.base.requestTileGeometry(x, y, level, request);
    }
    const [over, base] = await Promise.all([
      this.overlay.requestTileGeometry(x, y, level, undefined),
      this.baseTile(x, y, level, undefined),
    ]);
    if (!over) return base ? this.base.requestTileGeometry(x, y, level, request) : undefined;

    const scheme = this.tilingScheme;
    const rect = scheme.tileXYToRectangle(x, y, level);
    const baseRect = base ? scheme.tileXYToRectangle(base.x, base.y, base.level) : undefined;
    const n = this.gridSize;
    const heights = new Float32Array(n * n);
    for (let j = 0; j < n; j++) {
      const lat = rect.north - (rect.north - rect.south) * (j / (n - 1));
      for (let i = 0; i < n; i++) {
        const lon = rect.west + (rect.east - rect.west) * (i / (n - 1));
        let h = over.interpolateHeight(rect, lon, lat);
        if (h === undefined && base) h = base.data.interpolateHeight(baseRect, lon, lat);
        heights[j * n + i] = h === undefined ? this.fallbackHeight : h;
      }
    }
    return new Cesium.HeightmapTerrainData({
      buffer: heights,
      width: n,
      height: n,
      childTileMask: 15,
      structure: { heightScale: 1, heightOffset: 0, elementsPerHeight: 1, stride: 1, elementMultiplier: 256, isBigEndian: false },
      skirtHeight: 20,
      createdByUpsampling: false,
    });
  }
}
