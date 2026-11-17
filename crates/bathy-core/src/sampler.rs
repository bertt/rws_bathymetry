use crate::source::Source;
use crate::tiling::{Bounds, Heightmap};
use anyhow::Result;
use terrio::crs::{SpatialRef, Transformer};
use terrio::ops::Resampling;
use terrio::{FloatReadOptions, ResampledReadOptions, Window};

/// Samples ellipsoidal heights from NAP-referenced RD rasters at arbitrary lon/lat positions.
pub struct Sampler {
    sources: Vec<Source>,
    /// ETRS89 3D (lon, lat, h) -> RD New + NAP (x, y, H).
    to_rd: Transformer,
}

impl Sampler {
    pub fn new(mut sources: Vec<Source>) -> Result<Self> {
        sources.sort_by_key(|s| -s.priority);
        let to_rd = Transformer::new(&SpatialRef::parse("EPSG:4979")?, &SpatialRef::parse("EPSG:7415")?)?;
        Ok(Self { sources, to_rd })
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// Ellipsoidal height of NAP 0 m (the geoid undulation) at a position.
    pub fn nap_zero_height(&self, lon: f64, lat: f64) -> Result<f32> {
        let (_, _, z0) = self.to_rd.transform_3d(lon, lat, 0.0)?;
        Ok(-z0 as f32)
    }

    /// Geographic bounds (degrees) covered by all sources.
    pub fn geographic_bounds(&self) -> Result<Bounds> {
        let rd_to_geo = Transformer::new(&SpatialRef::parse("EPSG:28992")?, &SpatialRef::parse("EPSG:4258")?)?;
        let mut out: Option<Bounds> = None;
        for s in &self.sources {
            let (x0, y0, x1, y1) = s.extent;
            let mut b = Bounds { west: f64::MAX, south: f64::MAX, east: f64::MIN, north: f64::MIN };
            for i in 0..=20 {
                let t = f64::from(i) / 20.0;
                for (x, y) in [(x0 + (x1 - x0) * t, y0), (x0 + (x1 - x0) * t, y1), (x0, y0 + (y1 - y0) * t), (x1, y0 + (y1 - y0) * t)] {
                    let (lon, lat) = rd_to_geo.transform(x, y)?;
                    b = Bounds { west: b.west.min(lon), south: b.south.min(lat), east: b.east.max(lon), north: b.north.max(lat) };
                }
            }
            out = Some(out.map_or(b, |o| o.union(&b)));
        }
        out.ok_or_else(|| anyhow::anyhow!("no sources"))
    }

    /// Heightmap of `n` x `n` vertices across `b` (vertices on the tile edges).
    /// The first value is `None` if all vertices are nodata; the second tells whether any source
    /// has data inside the tile's neighbourhood (so finer levels may still contain data).
    pub fn heightmap(&self, b: &Bounds, n: usize) -> Result<(Option<Heightmap>, bool)> {
        let mut lons = Vec::with_capacity(n * n);
        let mut lats = Vec::with_capacity(n * n);
        for j in 0..n {
            let lat = b.south + (b.north - b.south) * j as f64 / (n - 1) as f64;
            for i in 0..n {
                lons.push(b.west + (b.east - b.west) * i as f64 / (n - 1) as f64);
                lats.push(lat);
            }
        }
        let spacing_m = (b.north - b.south) * 111_320.0 / (n - 1) as f64;
        let (heights, nearby) = self.sample_points(&lons, &lats, spacing_m)?;
        if heights.iter().all(|h| h.is_nan()) {
            return Ok((None, nearby));
        }
        Ok((Some(Heightmap { size: n, heights }), true))
    }

    /// Ellipsoidal height per point, NaN where no source has data, and whether any source window had data.
    /// `spacing_m` is the approximate distance between neighbouring points and selects the raster resolution.
    pub fn sample_points(&self, lons: &[f64], lats: &[f64], spacing_m: f64) -> Result<(Vec<f32>, bool)> {
        let mut nearby = false;
        let n = lons.len();
        let mut rd = Vec::with_capacity(n);
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for k in 0..n {
            let (x, y, z0) = self.to_rd.transform_3d(lons[k], lats[k], 0.0)?;
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
            rd.push((x, y, z0));
        }
        let mut out = vec![f32::NAN; n];
        for s in &self.sources {
            let (ex0, ey0, ex1, ey1) = s.extent;
            if minx >= ex1 || maxx <= ex0 || miny >= ey1 || maxy <= ey0 {
                continue;
            }
            let pad = 2.0 * s.pixel_size.max(spacing_m / 2.0);
            let (wx0, wy0) = ((minx - pad).max(ex0), (miny - pad).max(ey0));
            let (wx1, wy1) = ((maxx + pad).min(ex1), (maxy + pad).min(ey1));
            if wx1 <= wx0 || wy1 <= wy0 {
                continue;
            }
            let pix = (spacing_m / 2.0).max(s.pixel_size);
            let ow = (((wx1 - wx0) / pix).ceil() as usize).max(2);
            let oh = (((wy1 - wy0) / pix).ceil() as usize).max(2);
            let mut r = terrio::open_raster_lazy(&s.path.to_string_lossy())?;
            let gt = r.geo_transform();
            let opts = ResampledReadOptions {
                bands: vec![1],
                window: Some(((wx0 - gt[0]) / gt[1], (gt[3] - wy1) / gt[5].abs(), (wx1 - wx0) / gt[1], (wy1 - wy0) / gt[5].abs())),
                size: (ow, oh),
                resampling: Resampling::Average,
                ..Default::default()
            };
            let ds = r.read_resampled(&opts)?;
            let data = ds.read_window_f32(Window::full(ow, oh), &FloatReadOptions { bands: vec![1], ..Default::default() })?;
            let (px, py) = ((wx1 - wx0) / ow as f64, (wy1 - wy0) / oh as f64);
            nearby |= data.iter().any(|v| v.is_finite() && v.abs() < 1.0e6);
            let at = |c: i64, rr: i64| -> Option<f32> {
                if c < 0 || rr < 0 || c >= ow as i64 || rr >= oh as i64 {
                    return None;
                }
                let v = data[rr as usize * ow + c as usize];
                (v.is_finite() && v.abs() < 1.0e6).then_some(v)
            };
            for k in 0..n {
                if !out[k].is_nan() {
                    continue;
                }
                let (x, y, z0) = rd[k];
                let fx = (x - wx0) / px - 0.5;
                let fy = (wy1 - y) / py - 0.5;
                if at(fx.round() as i64, fy.round() as i64).is_none() {
                    continue;
                }
                let (c0, r0) = (fx.floor() as i64, fy.floor() as i64);
                let (tx, ty) = (fx - c0 as f64, fy - r0 as f64);
                let (mut sum, mut wsum) = (0.0f64, 0.0f64);
                for (dc, dr, w) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
                    if let Some(v) = at(c0 + dc, r0 + dr) {
                        sum += w * f64::from(v);
                        wsum += w;
                    }
                }
                if wsum > 0.0 {
                    // H(NAP) - z0, with z0 = -N, gives the ellipsoidal height.
                    out[k] = (sum / wsum - z0) as f32;
                }
            }
        }
        Ok((out, nearby))
    }
}
