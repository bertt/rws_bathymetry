use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// One entry of `data/sources.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceConfig {
    pub name: String,
    /// GeoTIFF path, relative to the registry file's directory.
    pub path: String,
    /// Horizontal+vertical CRS of the raster, e.g. "EPSG:7415" (RD New + NAP height).
    #[serde(default = "default_crs")]
    pub crs: String,
    /// Higher priority wins where sources overlap.
    #[serde(default)]
    pub priority: i32,
}

fn default_crs() -> String {
    "EPSG:7415".into()
}

#[derive(Debug, Deserialize)]
struct Registry {
    #[serde(rename = "source")]
    sources: Vec<SourceConfig>,
}

pub fn load_registry(file: &Path) -> Result<Vec<SourceConfig>> {
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let reg: Registry = toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
    let base = file.parent().unwrap_or(Path::new("."));
    Ok(reg
        .sources
        .into_iter()
        .map(|mut s| {
            s.path = base.join(&s.path).to_string_lossy().into_owned();
            s
        })
        .collect())
}

/// Metadata of an opened source; the raster itself is reopened per read so it can be used from many threads.
#[derive(Debug, Clone)]
pub struct Source {
    pub name: String,
    pub path: PathBuf,
    pub priority: i32,
    /// Extent in the raster's own (RD) coordinates: (min_x, min_y, max_x, max_y).
    pub extent: (f64, f64, f64, f64),
    pub pixel_size: f64,
}

impl Source {
    pub fn open(cfg: &SourceConfig) -> Result<Self> {
        let r = terrio::open_raster_lazy(&cfg.path).with_context(|| format!("opening {}", cfg.path))?;
        let gt = r.geo_transform();
        let (w, h) = (r.width() as f64, r.height() as f64);
        let (x0, y0) = (gt[0], gt[3]);
        let (x1, y1) = (gt[0] + gt[1] * w, gt[3] + gt[5] * h);
        Ok(Self {
            name: cfg.name.clone(),
            path: PathBuf::from(&cfg.path),
            priority: cfg.priority,
            extent: (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)),
            pixel_size: gt[1].abs(),
        })
    }
}
