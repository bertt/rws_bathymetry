# Extending the pipeline

## New input data

`data/sources.toml`:

```toml
[[source]]
name = "ON_oost_NAP"
path = "raw/ON_oost_NAP.tif"   # relative to sources.toml
crs = "EPSG:7415"   # horizontal EPSG:28992 + NAP heights
priority = 0        # higher wins where sources overlap
```

Any raster that terrio can read works. For a different vertical datum, add a transform in `bathy_core::vertical`
and select it from the source's `crs`.

## New output format

1. Create `crates/bathy-<format>` with an encoder. Use `Sampler::sample_points(lons, lats, spacing_m)` for ellipsoidal heights
   (NaN = no data) and `Sampler::nap_zero_height` to convert to NAP.
2. Add a `Format` variant and a `run_<format>` function in `crates/bathy-cli/src/main.rs`.

`bathy-terrarium` is the smallest example (Web Mercator XYZ tiles). A Mapbox Terrain-RGB encoder would differ only in the
RGB packing (`-10000 + (R*65536 + G*256 + B) * 0.1`).
