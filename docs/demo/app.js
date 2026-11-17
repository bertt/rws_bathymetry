// Approximate NAP -> ellipsoid offset (quasi-geoid height) around IJmuiden, in metres.
// Only used for colouring and contours; the terrain tiles themselves hold true ellipsoidal heights.
const GEOID_OFFSET = 42.5;
const PDOK = "https://api.pdok.nl/kadaster/3d-basisvoorziening/ogc/v1/collections/digitaalterreinmodel/quantized-mesh";

const RAMP_STOPS = [
  [0, "#c9f2f7"], [-2, "#6fd0e8"], [-5, "#2f94d6"], [-10, "#1d5fb5"], [-15, "#2a3a99"], [-25, "#1a1050"],
];

const status = document.getElementById("status");

// WGS84 -> RD New (EPSG:28992) with the Schreutelkamp/Strang van Hees polynomial approximation (about 0.5 m accuracy).
function toRD(latDeg, lonDeg) {
  const p = 0.36 * (latDeg - 52.15517440);
  const l = 0.36 * (lonDeg - 5.38720621);
  const x = 155000 + 190094.945 * l - 11832.228 * p * l - 114.221 * p * p * l - 32.391 * l ** 3
    - 0.705 * p - 2.340 * p ** 3 * l - 0.608 * p * l ** 3 - 0.008 * l * l + 0.148 * p * p * l ** 3;
  const y = 463000 + 309056.544 * p + 3638.893 * l * l + 73.077 * p * p - 157.984 * p * l * l
    + 59.788 * p ** 3 + 0.433 * l - 6.439 * p * p * l * l - 0.032 * p * l + 0.092 * l ** 4 - 0.054 * p * l ** 4;
  return [x, y];
}

const globeMaterial = new Cesium.Material({
  fabric: {
    uniforms: { offset: GEOID_OFFSET, spacing: 1.0, rampOn: 1.0, contourOn: 1.0 },
    source: `
      vec3 rampColor(float d) { // d = depth below NAP, metres
        vec3 c0 = vec3(0.79, 0.95, 0.97);
        vec3 c1 = vec3(0.44, 0.82, 0.91);
        vec3 c2 = vec3(0.18, 0.58, 0.84);
        vec3 c3 = vec3(0.11, 0.37, 0.71);
        vec3 c4 = vec3(0.16, 0.23, 0.60);
        vec3 c5 = vec3(0.10, 0.06, 0.31);
        if (d < 2.0) return mix(c0, c1, d / 2.0);
        if (d < 5.0) return mix(c1, c2, (d - 2.0) / 3.0);
        if (d < 10.0) return mix(c2, c3, (d - 5.0) / 5.0);
        if (d < 15.0) return mix(c3, c4, (d - 10.0) / 5.0);
        return mix(c4, c5, clamp((d - 15.0) / 10.0, 0.0, 1.0));
      }
      czm_material czm_getMaterial(czm_materialInput materialInput) {
        czm_material m = czm_getDefaultMaterial(materialInput);
        float nap = materialInput.height - offset;
        float alpha = 0.0;
        vec3 color = vec3(0.0);
        if (nap < 0.0 && rampOn > 0.5) {
          color = rampColor(-nap);
          alpha = 0.85;
        }
        if (contourOn > 0.5) {
          float dist = abs(nap - spacing * floor(nap / spacing + 0.5));
          float w = fwidth(nap) * 1.2;
          float line = 1.0 - smoothstep(0.0, w, dist);
          // every 5th contour is emphasised
          float major = 1.0 - smoothstep(0.0, w, abs(nap - 5.0 * floor(nap / 5.0 + 0.5)));
          vec3 lc = mix(vec3(1.0), vec3(1.0, 0.85, 0.2), major);
          float la = nap < 0.0 ? line * (0.5 + 0.5 * major) : 0.0;
          color = mix(color, lc, la);
          alpha = max(alpha, la);
        }
        m.diffuse = color;
        m.alpha = alpha;
        return m;
      }`,
  },
  translucent: true,
});

(async () => {
  const viewer = new Cesium.Viewer("cesiumContainer", {
    baseLayer: new Cesium.ImageryLayer(new Cesium.OpenStreetMapImageryProvider({ url: "https://tile.openstreetmap.org/" })),
    baseLayerPicker: false, geocoder: false, timeline: false, animation: false,
    sceneModePicker: false, navigationHelpButton: false, infoBox: false, selectionIndicator: false,
    terrainProvider: new Cesium.EllipsoidTerrainProvider(),
  });
  viewer.scene.globe.depthTestAgainstTerrain = true;
  viewer.scene.globe.enableLighting = false;
  viewer.scene.globe.material = globeMaterial;
  viewer.scene.globe.baseColor = Cesium.Color.fromCssColorString("#334");

  const url = new URL("terrain/", location.href).toString();
  let pdok, bathy;
  try {
    [pdok, bathy] = await Promise.all([
      Cesium.CesiumTerrainProvider.fromUrl(PDOK),
      Cesium.CesiumTerrainProvider.fromUrl(url),
    ]);
  } catch (e) {
    status.textContent = "Failed to load terrain: " + e;
    return;
  }
  const combined = new CompositeTerrainProvider(pdok, bathy, { fallbackHeight: GEOID_OFFSET });
  const providers = { combined, pdok, bathy };
  const setMode = (m) => { viewer.scene.terrainProvider = providers[m]; };
  setMode("combined");
  status.textContent = "Heights are WGS84 ellipsoid heights; colours/contours are relative to NAP.";

  document.querySelectorAll("input[name=mode]").forEach((r) =>
    r.addEventListener("change", (e) => setMode(e.target.value)));
  const u = globeMaterial.uniforms;
  document.getElementById("ramp").onchange = (e) => (u.rampOn = e.target.checked ? 1 : 0);
  document.getElementById("contours").onchange = (e) => (u.contourOn = e.target.checked ? 1 : 0);
  document.getElementById("spacing").onchange = (e) => (u.spacing = parseFloat(e.target.value));
  const alpha = document.getElementById("alpha");
  const setAlpha = () => (viewer.imageryLayers.get(0).alpha = parseFloat(alpha.value));
  alpha.oninput = setAlpha; setAlpha();

  const grad = RAMP_STOPS.map(([d, c]) => `${c} ${(-d / 25) * 100}%`).join(",");
  document.getElementById("legend").innerHTML =
    `<div class="bar" style="background:linear-gradient(to right,${grad})"></div>` +
    `<div class="ticks"><span>0 m NAP</span><span>−10</span><span>−25 m</span></div>`;

  const info = document.getElementById("info");
  const marker = viewer.entities.add({
    show: false,
    position: new Cesium.CallbackProperty(() => marker.pos, false),
    point: { pixelSize: 9, color: Cesium.Color.YELLOW, outlineColor: Cesium.Color.BLACK, outlineWidth: 2, disableDepthTestDistance: Number.POSITIVE_INFINITY },
  });
  const handler = new Cesium.ScreenSpaceEventHandler(viewer.scene.canvas);
  handler.setInputAction((click) => {
    const ray = viewer.camera.getPickRay(click.position);
    const pos = ray && viewer.scene.globe.pick(ray, viewer.scene);
    if (!pos) return;
    const c = Cesium.Cartographic.fromCartesian(pos);
    const nap = c.height - GEOID_OFFSET;
    const depth = nap < 0 ? `Depth: ${(-nap).toFixed(1)} m below NAP` : `Height: ${nap.toFixed(1)} m above NAP`;
    const lon = Cesium.Math.toDegrees(c.longitude), lat = Cesium.Math.toDegrees(c.latitude);
    const [rx, ry] = toRD(lat, lon);
    info.innerHTML = `${depth}<br>` +
      `${lon.toFixed(5)}°E, ${lat.toFixed(5)}°N<br>RD (EPSG:28992): X ${rx.toFixed(1)}, Y ${ry.toFixed(1)}`;
    marker.pos = pos;
    marker.show = true;
  }, Cesium.ScreenSpaceEventType.LEFT_CLICK);

  const target = Cesium.Cartesian3.fromDegrees(4.5575, 52.468, 0);
  viewer.camera.lookAt(target, new Cesium.HeadingPitchRange(Cesium.Math.toRadians(20), Cesium.Math.toRadians(-40), 5000));
  viewer.camera.lookAtTransform(Cesium.Matrix4.IDENTITY);
})();
