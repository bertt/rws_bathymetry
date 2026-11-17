//! Terrarium PNG encoder (Web Mercator XYZ tiles, e.g. for MapLibre `raster-dem` with `encoding: "terrarium"`).
//!
//! Elevation = (R * 256 + G + B / 256) - 32768 metres. Pixels without data get alpha 0.

use anyhow::Result;

pub const TILE_SIZE: usize = 256;
const MAX_LAT: f64 = 85.051_128_779_806_59;

/// A tile in the XYZ (slippy map) scheme, y counted from the north.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct XyzTile {
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

fn lon_of(x: f64, z: u32) -> f64 {
    x / f64::from(1u32 << z) * 360.0 - 180.0
}

fn lat_of(y: f64, z: u32) -> f64 {
    let n = std::f64::consts::PI * (1.0 - 2.0 * y / f64::from(1u32 << z));
    n.sinh().atan().to_degrees()
}

impl XyzTile {
    /// West, south, east, north in degrees.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let (x, y) = (f64::from(self.x), f64::from(self.y));
        (lon_of(x, self.z), lat_of(y + 1.0, self.z), lon_of(x + 1.0, self.z), lat_of(y, self.z))
    }

    /// Longitude/latitude of every pixel centre, row-major from the north-west.
    pub fn pixel_centres(&self, size: usize) -> (Vec<f64>, Vec<f64>) {
        let mut lons = Vec::with_capacity(size * size);
        let mut lats = Vec::with_capacity(size * size);
        for j in 0..size {
            let lat = lat_of(f64::from(self.y) + (j as f64 + 0.5) / size as f64, self.z);
            for i in 0..size {
                lons.push(lon_of(f64::from(self.x) + (i as f64 + 0.5) / size as f64, self.z));
                lats.push(lat);
            }
        }
        (lons, lats)
    }

    pub fn children(&self) -> [XyzTile; 4] {
        let (z, x, y) = (self.z + 1, self.x * 2, self.y * 2);
        [XyzTile { z, x, y }, XyzTile { z, x: x + 1, y }, XyzTile { z, x, y: y + 1 }, XyzTile { z, x: x + 1, y: y + 1 }]
    }

    /// Tiles at level `z` covering the bounds (clamped to the Web Mercator extent).
    pub fn covering(z: u32, w: f64, s: f64, e: f64, n: f64) -> Vec<XyzTile> {
        let count = 1u32 << z;
        let fx = |lon: f64| (((lon + 180.0) / 360.0 * f64::from(count)).floor().max(0.0) as u32).min(count - 1);
        let fy = |lat: f64| {
            let r = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
            let v = (1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * f64::from(count);
            (v.floor().max(0.0) as u32).min(count - 1)
        };
        let mut out = Vec::new();
        for y in fy(n)..=fy(s) {
            for x in fx(w)..=fx(e) {
                out.push(XyzTile { z, x, y });
            }
        }
        out
    }
}

/// Encode `size` x `size` heights (metres, NaN = no data) as an RGBA Terrarium PNG.
pub fn encode_png(size: usize, heights: &[f32]) -> Result<Vec<u8>> {
    let mut rgba = Vec::with_capacity(size * size * 4);
    for &h in heights {
        if h.is_nan() {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        let v = (f64::from(h) + 32768.0).clamp(0.0, 65535.99);
        let r = (v / 256.0).floor();
        let g = (v - r * 256.0).floor();
        let b = ((v - r * 256.0 - g) * 256.0).floor();
        rgba.extend_from_slice(&[r as u8, g as u8, b as u8, 255]);
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, size as u32, size as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgba)?;
    Ok(out)
}

/// TileJSON for the tile set.
pub fn tilejson(name: &str, attribution: &str, bounds: (f64, f64, f64, f64), min_zoom: u32, max_zoom: u32, datum: &str) -> serde_json::Value {
    serde_json::json!({
        "tilejson": "2.2.0",
        "name": name,
        "description": format!("Terrarium-encoded elevation, metres above {datum}"),
        "attribution": attribution,
        "scheme": "xyz",
        "encoding": "terrarium",
        "tileSize": TILE_SIZE,
        "tiles": ["{z}/{x}/{y}.png"],
        "minzoom": min_zoom,
        "maxzoom": max_zoom,
        "bounds": [bounds.0, bounds.1, bounds.2, bounds.3],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrarium_roundtrip() {
        let png_bytes = encode_png(1, &[-12.5]).unwrap();
        let dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size()];
        r.next_frame(&mut buf).unwrap();
        let h = f64::from(buf[0]) * 256.0 + f64::from(buf[1]) + f64::from(buf[2]) / 256.0 - 32768.0;
        assert!((h + 12.5).abs() < 0.01);
    }

    #[test]
    fn covering_finds_tile() {
        let t = XyzTile::covering(10, 4.56, 52.46, 4.561, 52.461);
        assert_eq!(t.len(), 1);
        let b = t[0].bounds();
        assert!(b.0 < 4.56 && b.2 > 4.561 && b.1 < 52.46 && b.3 > 52.461);
    }
}
