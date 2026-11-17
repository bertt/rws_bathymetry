# Combining two terrains in CesiumJS

`viewer.terrainProvider` accepts one provider. Options for bathymetry plus a land terrain such as PDOK:

1. **Composite provider (used in the demo).** `docs/demo/composite-terrain.js` requests both tiles, samples a 65×65 grid
   (bathymetry where defined, PDOK elsewhere) and returns a `HeightmapTerrainData` with ellipsoidal heights.
   Tiles without bathymetry are passed through from PDOK. Both tilesets must use ellipsoidal heights (they do).
   Cost: two requests per tile and a CPU resampling step; the result has a fixed 65×65 resolution.
2. **Merge at build time.** Add a land DTM as a lower-priority source in `data/sources.toml`, so the tool outputs one tileset.
   Fastest at runtime, but requires the land data as raster input.
3. **Separate bathymetry mesh.** Keep PDOK as terrain and draw the bathymetry as its own primitive/3D Tiles layer, clipping the globe
   with `ClippingPolygon`. Works, but loses globe-based material effects (ramp, contours).

Notes:

* Per-tile switching between two providers does not work, because our tiles are water-only and have holes.
* PDOK tiles stop at level 15; for deeper levels the composite provider uses the level-15 ancestor for the land values.
* PDOK responds with HTTP 204 for tiles outside its coverage (for example far offshore); the composite falls back to NAP 0 there.
