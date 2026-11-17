use anyhow::Result;
use terrio::crs::{SpatialRef, Transformer};

/// Converts RD New + NAP heights (EPSG:7415) to ETRS89 geographic 3D (EPSG:4979)
/// using the RDNAPTRANS / NLGEO2018 grids.
pub struct NapToEllipsoid {
    t: Transformer,
}

/// Points terrio at a directory holding `nl_nsgi_nlgeo2018.tif` and `nl_nsgi_rdtrans2018.tif`.
pub fn set_grid_dir(dir: impl Into<std::path::PathBuf>) {
    terrio::crs::set_grid_search_paths(Some(vec![dir.into()]));
}

impl NapToEllipsoid {
    pub fn new() -> Result<Self> {
        let src = SpatialRef::parse("EPSG:7415")?;
        let dst = SpatialRef::parse("EPSG:4979")?;
        Ok(Self { t: Transformer::new(&src, &dst)? })
    }

    /// (x, y, h_nap) in RD metres -> (lon, lat, h_ellipsoid).
    pub fn forward(&self, x: f64, y: f64, h: f64) -> Result<(f64, f64, f64)> {
        Ok(self.t.transform_3d(x, y, h)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amersfoort_geoid_separation() {
        set_grid_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/grids"));
        let t = NapToEllipsoid::new().unwrap();
        let (lon, lat, h) = t.forward(155000.0, 463000.0, 0.0).unwrap();
        println!("{lon} {lat} {h}");
        assert!((lon - 5.3872).abs() < 1e-3 && (lat - 52.1552).abs() < 1e-3);
        assert!((h - 43.0).abs() < 1.5, "geoid separation was {h}");
    }
}
