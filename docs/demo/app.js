// Approximate NAP -> ellipsoid offset (quasi-geoid height) around IJmuiden, in metres.
// Only used for colouring and contours; the terrain tiles themselves hold true ellipsoidal heights.
const GEOID_OFFSET = 42.5;
const PDOK = "https://api.pdok.nl/kadaster/3d-basisvoorziening/ogc/v1/collections/digitaalterreinmodel/quantized-mesh";

// Depth (m below NAP) of the six colour stops; the selected ramp supplies one colour per stop.
const RAMP_DEPTHS = [0, 2, 5, 10, 15, 25];
const RAMPS = {
  "Blues": ["#c9f2f7", "#6fd0e8", "#2f94d6", "#1d5fb5", "#2a3a99", "#1a1050"],
  "Viridis": ["#fde725", "#7ad151", "#22a884", "#2a788e", "#414487", "#440154"],
  "Turbo (rainbow)": ["#a4fc3c", "#e2dc38", "#fe9b2d", "#e1480b", "#8a1d9a", "#30123b"],
  "Nautical (shallow = warm)": ["#f2e6b1", "#f2b36b", "#6fb98f", "#3b8fb5", "#2a5a9a", "#16264d"],
  "Grayscale": ["#f2f2f2", "#cfcfcf", "#a0a0a0", "#707070", "#404040", "#101010"],
};

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
    uniforms: { offset: GEOID_OFFSET, spacing: 1.0, rampOn: 1.0, contourOn: 1.0,
      c0: Cesium.Color.WHITE, c1: Cesium.Color.WHITE, c2: Cesium.Color.WHITE, c3: Cesium.Color.WHITE, c4: Cesium.Color.WHITE, c5: Cesium.Color.WHITE },
    source: `
      vec3 rampColor(float d) { // d = depth below NAP, metres
        if (d < 2.0) return mix(c0.rgb, c1.rgb, d / 2.0);
        if (d < 5.0) return mix(c1.rgb, c2.rgb, (d - 2.0) / 3.0);
        if (d < 10.0) return mix(c2.rgb, c3.rgb, (d - 5.0) / 5.0);
        if (d < 15.0) return mix(c3.rgb, c4.rgb, (d - 10.0) / 5.0);
        return mix(c4.rgb, c5.rgb, clamp((d - 15.0) / 10.0, 0.0, 1.0));
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

  // Exaggerate relative to NAP 0 so the water level stays put and the seabed gets deeper.
  viewer.scene.verticalExaggerationRelativeHeight = GEOID_OFFSET;
  const exag = document.getElementById("exag");
  exag.oninput = () => {
    viewer.scene.verticalExaggeration = parseFloat(exag.value);
    document.getElementById("exagVal").textContent = exag.value + "×";
  };

  const legend = document.getElementById("legend");
  const rampSelect = document.getElementById("rampSelect");
  Object.keys(RAMPS).forEach((name) => rampSelect.add(new Option(name, name)));
  const setRamp = () => {
    const colors = RAMPS[rampSelect.value];
    colors.forEach((c, i) => (u["c" + i] = Cesium.Color.fromCssColorString(c)));
    const grad = colors.map((c, i) => `${c} ${(RAMP_DEPTHS[i] / 25) * 100}%`).join(",");
    legend.innerHTML =
      `<div class="bar" style="background:linear-gradient(to right,${grad})"></div>` +
      `<div class="ticks"><span>0 m NAP</span><span>−10</span><span>−25 m</span></div>`;
  };
  rampSelect.onchange = setRamp;
  setRamp();

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
      `RD (EPSG:28992): X ${rx.toFixed(1)}, Y ${ry.toFixed(1)}`;
    marker.pos = pos;
    marker.show = true;
  }, Cesium.ScreenSpaceEventType.LEFT_CLICK);

  const target = Cesium.Cartesian3.fromDegrees(4.5575, 52.468, 0);
  viewer.camera.lookAt(target, new Cesium.HeadingPitchRange(Cesium.Math.toRadians(20), Cesium.Math.toRadians(-40), 5000));
  viewer.camera.lookAtTransform(Cesium.Matrix4.IDENTITY);
})();
