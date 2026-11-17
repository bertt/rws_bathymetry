mod download;

use anyhow::{Context, Result, bail};
use bathy_core::sampler::Sampler;
use bathy_core::source::{Source, load_registry};
use bathy_core::tiling::{Bounds, TileId};
use clap::{Parser, Subcommand, ValueEnum};
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "bathy", about = "Rijkswaterstaat bathymetry -> terrain tiles")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download the 1 m bathymetry GeoTIFFs (NAP variant) and register them in sources.toml.
    Download {
        #[arg(long, default_value = download::DEFAULT_BASE)]
        base: String,
        #[arg(long, default_value = "data/raw")]
        out: PathBuf,
        /// File name suffix to download; NAP files are referenced to NAP, LAT files to Lowest Astronomical Tide.
        #[arg(long, default_value = "_NAP.tif")]
        suffix: String,
        /// Only download areas whose name contains one of these strings (e.g. ON_oost).
        #[arg(long)]
        only: Vec<String>,
        #[arg(long, default_value = "data/sources.toml")]
        registry: PathBuf,
        /// List the files without downloading.
        #[arg(long)]
        list: bool,
    },
    /// Download the Dutch geoid / RD transformation grids used for NAP -> ellipsoid height conversion.
    FetchGrids {
        #[arg(long, default_value = "data/grids")]
        out: PathBuf,
    },
    /// Download all GeoTIFFs, then write GeoJSON polygons of the areas that contain data.
    Coverage {
        #[arg(long, default_value = download::DEFAULT_BASE)]
        base: String,
        #[arg(long, default_value = "data/raw")]
        raw: PathBuf,
        #[arg(long, default_value = "_NAP.tif")]
        suffix: String,
        #[arg(long)]
        only: Vec<String>,
        #[arg(long, default_value = "data/sources.toml")]
        registry: PathBuf,
        /// Use the files already registered in the registry; do not download.
        #[arg(long)]
        skip_download: bool,
        /// Footprint cell size in pixels (e.g. 32 = 32 x 32 m cells for 1 m data). Smaller is more detailed.
        #[arg(long, default_value_t = 32)]
        cell: usize,
        #[arg(long, default_value = "data/coverage.geojson")]
        out: PathBuf,
        #[arg(long, default_value = "data/grids")]
        grids: PathBuf,
    },
    /// Create terrain tiles from the registered sources.
    Tile {
        #[arg(long, default_value = "data/sources.toml")]
        sources: PathBuf,
        #[arg(long, default_value = "output/quantized-mesh")]
        out: PathBuf,
        #[arg(long, value_enum, default_value = "quantized-mesh")]
        format: Format,
        #[arg(long, default_value_t = 17)]
        max_zoom: u32,
        /// Vertices per tile edge.
        #[arg(long, default_value_t = 65)]
        grid: usize,
        /// Restrict to a geographic bounding box: west,south,east,north (degrees).
        #[arg(long, value_delimiter = ',')]
        bbox: Option<Vec<f64>>,
        /// Gzip tiles (serve them with `Content-Encoding: gzip`; GitHub Pages cannot, so leave off there).
        #[arg(long)]
        gzip: bool,
        #[arg(long, default_value = "data/grids")]
        grids: PathBuf,
        /// Terrarium only: heights relative to NAP (default, matches MapLibre basemaps) or the ellipsoid.
        #[arg(long, value_enum, default_value = "nap")]
        datum: Datum,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    QuantizedMesh,
    Terrarium,
}

/// Vertical reference of Terrarium output.
#[derive(Clone, Copy, ValueEnum, PartialEq)]
enum Datum {
    Nap,
    Ellipsoid,
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Download { base, out, suffix, only, registry, list } => run_download(&base, &out, &suffix, &only, &registry, list),
        Cmd::FetchGrids { out } => {
            fs::create_dir_all(&out)?;
            for g in ["nl_nsgi_nlgeo2018.tif", "nl_nsgi_rdtrans2018.tif"] {
                download::download_file(&format!("https://cdn.proj.org/{g}"), &out.join(g))?;
            }
            Ok(())
        }
        Cmd::Coverage { base, raw, suffix, only, registry, skip_download, cell, out, grids } => {
            if !skip_download {
                run_download(&base, &raw, &suffix, &only, &registry, false)?;
            }
            run_coverage(&registry, &only, cell, &out, &grids)
        }
        Cmd::Tile { sources, out, format, max_zoom, grid, bbox, gzip, grids, datum } => match format {
            Format::QuantizedMesh => run_tile(&sources, &out, max_zoom, grid, bbox, gzip, &grids),
            Format::Terrarium => run_terrarium(&sources, &out, max_zoom, bbox, &grids, datum),
        },
    }
}

fn open_sampler(sources: &Path, bbox: Option<Vec<f64>>, grids: &Path) -> Result<(Sampler, Bounds)> {
    bathy_core::vertical::set_grid_dir(grids);
    let cfgs = load_registry(sources)?;
    let srcs = cfgs.iter().map(Source::open).collect::<Result<Vec<_>>>().context("opening sources")?;
    println!("{} sources", srcs.len());
    let sampler = Sampler::new(srcs)?;
    let bounds = match bbox {
        Some(b) if b.len() == 4 => Bounds { west: b[0], south: b[1], east: b[2], north: b[3] },
        Some(_) => bail!("--bbox needs west,south,east,north"),
        None => sampler.geographic_bounds()?,
    };
    println!("bounds {:.5},{:.5} .. {:.5},{:.5}", bounds.west, bounds.south, bounds.east, bounds.north);
    Ok((sampler, bounds))
}

fn run_terrarium(sources: &Path, out: &Path, max_zoom: u32, bbox: Option<Vec<f64>>, grids: &Path, datum: Datum) -> Result<()> {
    use bathy_terrarium::{TILE_SIZE, XyzTile};
    let (sampler, bounds) = open_sampler(sources, bbox, grids)?;
    fs::create_dir_all(out)?;
    let mut level = XyzTile::covering(0, bounds.west, bounds.south, bounds.east, bounds.north);
    let mut written_total = 0usize;
    let mut top = 0;
    for z in 0..=max_zoom {
        let results: Vec<(XyzTile, bool, bool)> = level
            .par_iter()
            .map(|t| -> Result<(XyzTile, bool, bool)> {
                let (lons, lats) = t.pixel_centres(TILE_SIZE);
                let (w, s, e, n) = t.bounds();
                let spacing = (n - s) * 111_320.0 / TILE_SIZE as f64;
                let (mut heights, nearby) = sampler.sample_points(&lons, &lats, spacing)?;
                if heights.iter().all(|h| h.is_nan()) {
                    return Ok((*t, false, nearby));
                }
                if datum == Datum::Nap {
                    // Ellipsoid height -> NAP, using the geoid height at the tile centre (varies by cm per tile).
                    let n0 = sampler.nap_zero_height((w + e) / 2.0, (s + n) / 2.0)?;
                    heights.iter_mut().for_each(|h| *h -= n0);
                }
                let dir = out.join(t.z.to_string()).join(t.x.to_string());
                fs::create_dir_all(&dir)?;
                fs::write(dir.join(format!("{}.png", t.y)), bathy_terrarium::encode_png(TILE_SIZE, &heights)?)?;
                Ok((*t, true, true))
            })
            .collect::<Result<_>>()?;
        let written = results.iter().filter(|r| r.1).count();
        println!("level {z}: {} tiles checked, {written} written", results.len());
        if written > 0 {
            top = z;
        }
        written_total += written;
        let next: BTreeSet<XyzTile> = results.iter().filter(|r| r.2 && z < max_zoom).flat_map(|r| r.0.children()).collect();
        level = next
            .into_iter()
            .filter(|c| {
                let (w, s, e, n) = c.bounds();
                bounds.intersects(&Bounds { west: w, south: s, east: e, north: n })
            })
            .collect();
        if level.is_empty() {
            break;
        }
    }
    let datum_name = if datum == Datum::Nap { "NAP" } else { "the WGS84/ETRS89 ellipsoid" };
    let tj = bathy_terrarium::tilejson("RWS bathymetry", "Rijkswaterstaat, Landelijke Opslag Lodingen (CC0)", (bounds.west, bounds.south, bounds.east, bounds.north), 0, top, datum_name);
    fs::write(out.join("tiles.json"), serde_json::to_string_pretty(&tj)?)?;
    println!("wrote {written_total} tiles + tiles.json to {}", out.display());
    Ok(())
}

fn run_coverage(registry: &Path, only: &[String], cell: usize, out: &Path, grids: &Path) -> Result<()> {
    use terrio::crs::{SpatialRef, Transformer};
    if cell == 0 {
        bail!("--cell must be at least 1");
    }
    bathy_core::vertical::set_grid_dir(grids);
    let to_wgs84 = Transformer::new(&SpatialRef::parse("EPSG:28992")?, &SpatialRef::parse("EPSG:4326")?)?;
    let round = |v: f64| (v * 1e7).round() / 1e7;
    let mut features = Vec::new();
    for cfg in load_registry(registry)?.iter().filter(|c| only.is_empty() || only.iter().any(|o| c.name.contains(o.as_str()))) {
        let src = Source::open(cfg)?;
        let comps = bathy_core::coverage::footprint(&src, cell)?;
        let res = src.pixel_size;
        println!("{}: {} areas, {} valid cells", src.name, comps.len(), comps.iter().map(|c| c.valid_cells).sum::<u64>());
        for c in comps {
            let mut polys = Vec::new();
            for poly in &c.polygons {
                let mut rings = Vec::new();
                for ring in poly {
                    let mut coords = Vec::with_capacity(ring.len() + 1);
                    for &(x, y) in ring {
                        let (lon, lat) = to_wgs84.transform(x, y)?;
                        coords.push(serde_json::json!([round(lon), round(lat)]));
                    }
                    coords.push(coords[0].clone());
                    rings.push(coords);
                }
                polys.push(rings);
            }
            let geometry = if polys.len() == 1 {
                serde_json::json!({ "type": "Polygon", "coordinates": polys[0] })
            } else {
                serde_json::json!({ "type": "MultiPolygon", "coordinates": polys })
            };
            features.push(serde_json::json!({
                "type": "Feature",
                "geometry": geometry,
                "properties": {
                    "source": src.name,
                    "resolution_m": res,
                    "valid_cells": c.valid_cells,
                    "area_m2": c.valid_cells as f64 * res * res,
                    "footprint_cell_m": cell as f64 * res,
                    "footprint_cells": c.footprint_cells,
                }
            }));
        }
    }
    let fc = serde_json::json!({ "type": "FeatureCollection", "features": features });
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(out, serde_json::to_string(&fc)?)?;
    println!("wrote {} features to {}", fc["features"].as_array().map_or(0, Vec::len), out.display());
    Ok(())
}

fn run_download(base: &str, out: &Path, suffix: &str, only: &[String], registry: &Path, list: bool) -> Result<()> {
    let base = if base.ends_with('/') { base.to_owned() } else { format!("{base}/") };
    let names: Vec<String> = download::list_tifs(&base, suffix)?
        .into_iter()
        .filter(|n| only.is_empty() || only.iter().any(|o| n.contains(o.as_str())))
        .collect();
    if names.is_empty() {
        bail!("no files matching {suffix} found at {base}");
    }
    if list {
        names.iter().for_each(|n| println!("{n}"));
        return Ok(());
    }
    fs::create_dir_all(out)?;
    let mut done = Vec::new();
    for name in &names {
        let dest = out.join(name);
        download::download_file(&format!("{base}{name}"), &dest)?;
        // The ISO 19115 XML is small and documents the dataset.
        let _ = download::download_file(&format!("{base}{name}.xml"), &out.join(format!("{name}.xml")));
        done.push(dest);
    }
    download::update_registry(registry, &done)?;
    println!("registered {} sources in {}", done.len(), registry.display());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_tile(sources: &Path, out: &Path, max_zoom: u32, grid: usize, bbox: Option<Vec<f64>>, gzip: bool, grids: &Path) -> Result<()> {
    let (sampler, bounds) = open_sampler(sources, bbox, grids)?;
    fs::create_dir_all(out)?;

    let opts = bathy_qm::EncodeOptions { gzip };
    let mut level: Vec<TileId> = TileId::covering(0, &bounds);
    let mut all: Vec<TileId> = Vec::new();
    for z in 0..=max_zoom {
        let results: Vec<(TileId, bool, bool)> = level
            .par_iter()
            .map(|t| -> Result<(TileId, bool, bool)> {
                let (hm, nearby) = sampler.heightmap(&t.bounds(), grid)?;
                let mut written = false;
                if let Some(hm) = hm {
                    if let Some(bytes) = bathy_qm::encode_tile(t, &hm, &opts)? {
                        let dir = out.join(t.z.to_string()).join(t.x.to_string());
                        fs::create_dir_all(&dir)?;
                        fs::write(dir.join(format!("{}.terrain", t.y)), bytes)?;
                        written = true;
                    }
                }
                Ok((*t, written, nearby))
            })
            .collect::<Result<_>>()?;
        let written = results.iter().filter(|r| r.1).count();
        println!("level {z}: {} tiles checked, {written} written", results.len());
        all.extend(results.iter().filter(|r| r.1).map(|r| r.0));
        let next: BTreeSet<TileId> = results.iter().filter(|r| r.2 && z < max_zoom).flat_map(|r| r.0.children()).filter(|c| c.bounds().intersects(&bounds)).collect();
        level = next.into_iter().collect();
        if level.is_empty() {
            break;
        }
    }
    // Cesium assumes every ancestor of an available tile exists. Fill gaps with flat sea-level tiles
    // (3x3 vertices, min == max) which viewers can recognise and ignore.
    let present: BTreeSet<TileId> = all.iter().copied().collect();
    let mut missing: BTreeSet<TileId> = BTreeSet::new();
    for t in &all {
        let (mut z, mut x, mut y) = (t.z, t.x, t.y);
        while z > 0 {
            (z, x, y) = (z - 1, x / 2, y / 2);
            let p = TileId { z, x, y };
            if !present.contains(&p) && !missing.insert(p) {
                break;
            }
        }
    }
    for t in &missing {
        let b = t.bounds();
        let h = sampler.nap_zero_height((b.west + b.east) / 2.0, (b.south + b.north) / 2.0)?;
        let hm = bathy_core::tiling::Heightmap { size: 3, heights: vec![h; 9] };
        let bytes = bathy_qm::encode_tile(t, &hm, &opts)?.expect("flat tile has triangles");
        let dir = out.join(t.z.to_string()).join(t.x.to_string());
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(format!("{}.terrain", t.y)), bytes)?;
    }
    println!("{} placeholder ancestor tiles", missing.len());
    all.extend(missing);

    let top = all.iter().map(|t| t.z).max().unwrap_or(0);
    let layer = bathy_qm::layer_json(&bounds, &all, top, "RWS bathymetry", "Rijkswaterstaat, Landelijke Opslag Lodingen (CC0)");
    fs::write(out.join("layer.json"), serde_json::to_string_pretty(&layer)?)?;
    println!("wrote {} tiles + layer.json to {}", all.len(), out.display());
    Ok(())
}
