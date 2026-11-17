//! Cesium quantized-mesh-1.0 encoder.
use anyhow::Result;
use bathy_core::tiling::{Bounds, Heightmap, TileId};
use flate2::{Compression, write::GzEncoder};
use std::io::Write;

const WGS84_A: f64 = 6_378_137.0;
const WGS84_B: f64 = 6_356_752.314_245_179;
const MAX: f64 = 32767.0;

type V3 = [f64; 3];

fn ecef(lon_deg: f64, lat_deg: f64, h: f64) -> V3 {
    let (lon, lat) = (lon_deg.to_radians(), lat_deg.to_radians());
    let e2 = 1.0 - (WGS84_B * WGS84_B) / (WGS84_A * WGS84_A);
    let n = WGS84_A / (1.0 - e2 * lat.sin().powi(2)).sqrt();
    [(n + h) * lat.cos() * lon.cos(), (n + h) * lat.cos() * lon.sin(), (n * (1.0 - e2) + h) * lat.sin()]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn scaled(p: V3) -> V3 {
    [p[0] / WGS84_A, p[1] / WGS84_A, p[2] / WGS84_B]
}

/// Cesium's EllipsoidalOccluder.computeHorizonCullingPoint, in scaled space.
fn horizon_point(center: V3, positions: &[V3]) -> V3 {
    let s = scaled(center);
    let dir = {
        let m = norm(s);
        [s[0] / m, s[1] / m, s[2] / m]
    };
    let mut max_mag = 0.0f64;
    for p in positions {
        let sp = scaled(*p);
        let mag2 = dot(sp, sp);
        let mag = mag2.sqrt();
        let d = [sp[0] / mag, sp[1] / mag, sp[2] / mag];
        let cos_alpha = dot(d, dir);
        let sin_alpha = norm(cross(d, dir));
        let cos_beta = 1.0 / mag;
        let sin_beta = (mag2 - 1.0).max(0.0).sqrt() * cos_beta;
        let m = 1.0 / (cos_alpha * cos_beta - sin_alpha * sin_beta);
        max_mag = max_mag.max(m);
    }
    [dir[0] * max_mag, dir[1] * max_mag, dir[2] * max_mag]
}

fn zigzag(v: i32) -> u16 {
    ((v << 1) ^ (v >> 31)) as u16
}

pub struct EncodeOptions {
    pub gzip: bool,
}

/// Encodes a heightmap as a quantized-mesh tile. Triangles touching nodata vertices are dropped.
/// Returns `None` when no triangle remains.
pub fn encode_tile(tile: &TileId, hm: &Heightmap, opts: &EncodeOptions) -> Result<Option<Vec<u8>>> {
    let n = hm.size;
    let b = tile.bounds();
    let valid = |i: usize, j: usize| !hm.heights[j * n + i].is_nan();

    let mut tris: Vec<[usize; 3]> = Vec::new();
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let (a, bb, c, d) = (j * n + i, j * n + i + 1, (j + 1) * n + i, (j + 1) * n + i + 1);
            if valid(i, j) && valid(i + 1, j) && valid(i + 1, j + 1) {
                tris.push([a, bb, d]);
            }
            if valid(i, j) && valid(i + 1, j + 1) && valid(i, j + 1) {
                tris.push([a, d, c]);
            }
        }
    }
    if tris.is_empty() {
        return Ok(None);
    }

    // Keep only vertices used by a triangle.
    let mut remap = vec![u32::MAX; n * n];
    let mut verts: Vec<usize> = Vec::new();
    for t in &tris {
        for &v in t {
            if remap[v] == u32::MAX {
                remap[v] = verts.len() as u32;
                verts.push(v);
            }
        }
    }
    let (mut hmin, mut hmax) = (f32::MAX, f32::MIN);
    for &v in &verts {
        hmin = hmin.min(hm.heights[v]);
        hmax = hmax.max(hm.heights[v]);
    }
    let range = f64::from((hmax - hmin).max(1e-3));

    let vc = verts.len();
    let mut us = Vec::with_capacity(vc);
    let mut vs = Vec::with_capacity(vc);
    let mut hs = Vec::with_capacity(vc);
    let mut positions = Vec::with_capacity(vc);
    for &v in &verts {
        let (i, j) = (v % n, v / n);
        us.push((i as f64 / (n - 1) as f64 * MAX).round() as i32);
        vs.push((j as f64 / (n - 1) as f64 * MAX).round() as i32);
        hs.push(((f64::from(hm.heights[v] - hmin) / range) * MAX).round() as i32);
        let lon = b.west + (b.east - b.west) * i as f64 / (n - 1) as f64;
        let lat = b.south + (b.north - b.south) * j as f64 / (n - 1) as f64;
        positions.push(ecef(lon, lat, f64::from(hm.heights[v])));
    }

    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in &positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let sphere_c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let radius = positions.iter().map(|p| norm([p[0] - sphere_c[0], p[1] - sphere_c[1], p[2] - sphere_c[2]])).fold(0.0, f64::max);
    let center = ecef((b.west + b.east) / 2.0, (b.south + b.north) / 2.0, f64::from(hmin + hmax) / 2.0);
    let horizon = horizon_point(sphere_c, &positions);

    let mut out: Vec<u8> = Vec::with_capacity(88 + vc * 6 + tris.len() * 6);
    let put_f64 = |o: &mut Vec<u8>, vals: &[f64]| vals.iter().for_each(|v| o.extend_from_slice(&v.to_le_bytes()));
    put_f64(&mut out, &center);
    out.extend_from_slice(&hmin.to_le_bytes());
    out.extend_from_slice(&hmax.to_le_bytes());
    put_f64(&mut out, &sphere_c);
    put_f64(&mut out, &[radius]);
    put_f64(&mut out, &horizon);

    out.extend_from_slice(&(vc as u32).to_le_bytes());
    for arr in [&us, &vs, &hs] {
        let mut prev = 0;
        for &v in arr {
            out.extend_from_slice(&zigzag(v - prev).to_le_bytes());
            prev = v;
        }
    }

    let wide = vc > 65536;
    if wide {
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    let put_index = |o: &mut Vec<u8>, v: u32| {
        if wide {
            o.extend_from_slice(&v.to_le_bytes());
        } else {
            o.extend_from_slice(&(v as u16).to_le_bytes());
        }
    };
    let mut highest = 0u32;
    for t in &tris {
        for &v in t {
            let idx = remap[v];
            put_index(&mut out, highest - idx);
            if idx == highest {
                highest += 1;
            }
        }
    }

    // Edge vertices, ordered along each edge.
    let edge = |pred: &dyn Fn(usize, usize) -> bool, key: &dyn Fn(usize, usize) -> usize| -> Vec<u32> {
        let mut list: Vec<(usize, u32)> = verts
            .iter()
            .enumerate()
            .filter(|(_, v)| pred(**v % n, **v / n))
            .map(|(k, v)| (key(*v % n, *v / n), k as u32))
            .collect();
        list.sort();
        list.into_iter().map(|(_, k)| k).collect()
    };
    let west = edge(&|i, _| i == 0, &|_, j| j);
    let south = edge(&|_, j| j == 0, &|i, _| i);
    let east = edge(&|i, _| i == n - 1, &|_, j| j);
    let north = edge(&|_, j| j == n - 1, &|i, _| i);
    for list in [&west, &south, &east, &north] {
        out.extend_from_slice(&(list.len() as u32).to_le_bytes());
        for &v in list {
            put_index(&mut out, v);
        }
    }

    if opts.gzip {
        let mut enc = GzEncoder::new(Vec::new(), Compression::best());
        enc.write_all(&out)?;
        return Ok(Some(enc.finish()?));
    }
    Ok(Some(out))
}

/// Builds `layer.json`; `tiles` is the set of tiles present at every level.
pub fn layer_json(bounds: &Bounds, tiles: &[TileId], max_zoom: u32, name: &str, attribution: &str) -> serde_json::Value {
    let mut available = Vec::new();
    for z in 0..=max_zoom {
        let mut level: Vec<&TileId> = tiles.iter().filter(|t| t.z == z).collect();
        level.sort_by_key(|t| (t.y, t.x));
        // One range per contiguous run of x within a row of y.
        let mut ranges = Vec::new();
        let mut i = 0;
        while i < level.len() {
            let (y, start) = (level[i].y, level[i].x);
            let mut end = start;
            let mut k = i + 1;
            while k < level.len() && level[k].y == y && level[k].x == end + 1 {
                end = level[k].x;
                k += 1;
            }
            ranges.push(serde_json::json!({"startX": start, "endX": end, "startY": y, "endY": y}));
            i = k;
        }
        available.push(ranges);
    }
    serde_json::json!({
        "tilejson": "2.1.0",
        "name": name,
        "description": "Quantized-mesh terrain, heights above the WGS84/ETRS89 ellipsoid",
        "version": "1.0.0",
        "format": "quantized-mesh-1.0",
        "attribution": attribution,
        "scheme": "tms",
        "projection": "EPSG:4326",
        "tiles": ["{z}/{x}/{y}.terrain"],
        "minzoom": 0,
        "maxzoom": max_zoom,
        "bounds": [bounds.west, bounds.south, bounds.east, bounds.north],
        "available": available,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_tile_encodes() {
        let hm = Heightmap { size: 5, heights: vec![10.0; 25] };
        let t = TileId { z: 10, x: 1000, y: 700 };
        let bytes = encode_tile(&t, &hm, &EncodeOptions { gzip: false }).unwrap().unwrap();
        // header 88 + vertex count 4 + 3*25*2 + triangle count 4 + 32 triangles*3*2 + four edge lists (count + 5 indices)
        assert_eq!(bytes.len(), 88 + 4 + 150 + 4 + 192 + 4 * (4 + 10));
    }

    #[test]
    fn nodata_tile_is_none() {
        let hm = Heightmap { size: 3, heights: vec![f32::NAN; 9] };
        assert!(encode_tile(&TileId { z: 1, x: 0, y: 0 }, &hm, &EncodeOptions { gzip: false }).unwrap().is_none());
    }
}
