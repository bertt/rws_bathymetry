/// A tile in the Cesium "geodetic" TMS scheme (EPSG:4326, 2x1 tiles at level 0, y counted from the south).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

/// Geographic bounds in degrees.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl TileId {
    pub fn size_deg(z: u32) -> f64 {
        180.0 / f64::from(1u32 << z)
    }

    pub fn bounds(&self) -> Bounds {
        let s = Self::size_deg(self.z);
        Bounds {
            west: -180.0 + f64::from(self.x) * s,
            south: -90.0 + f64::from(self.y) * s,
            east: -180.0 + f64::from(self.x + 1) * s,
            north: -90.0 + f64::from(self.y + 1) * s,
        }
    }

    pub fn children(&self) -> [TileId; 4] {
        let (z, x, y) = (self.z + 1, self.x * 2, self.y * 2);
        [
            TileId { z, x, y },
            TileId { z, x: x + 1, y },
            TileId { z, x, y: y + 1 },
            TileId { z, x: x + 1, y: y + 1 },
        ]
    }

    /// Level-`z` tiles covering `b`, clamped to the valid tile range.
    pub fn covering(z: u32, b: &Bounds) -> Vec<TileId> {
        let s = Self::size_deg(z);
        let nx = 2u32 << z;
        let ny = 1u32 << z;
        let clamp = |v: f64, max: u32| (v.floor().max(0.0) as u32).min(max - 1);
        let x0 = clamp((b.west + 180.0) / s, nx);
        let x1 = clamp((b.east + 180.0) / s, nx);
        let y0 = clamp((b.south + 90.0) / s, ny);
        let y1 = clamp((b.north + 90.0) / s, ny);
        let mut out = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                out.push(TileId { z, x, y });
            }
        }
        out
    }
}

impl Bounds {
    pub fn intersects(&self, o: &Bounds) -> bool {
        self.west < o.east && self.east > o.west && self.south < o.north && self.north > o.south
    }

    pub fn union(&self, o: &Bounds) -> Bounds {
        Bounds {
            west: self.west.min(o.west),
            south: self.south.min(o.south),
            east: self.east.max(o.east),
            north: self.north.max(o.north),
        }
    }
}

/// A heightmap sampled on a regular lon/lat grid; row 0 is the southern edge.
/// Heights are metres above the ellipsoid, NaN where there is no data.
pub struct Heightmap {
    pub size: usize,
    pub heights: Vec<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level0_has_two_tiles() {
        let b = Bounds { west: -180.0, south: -90.0, east: 180.0, north: 90.0 };
        assert_eq!(TileId::covering(0, &b).len(), 2);
    }

    #[test]
    fn bounds_roundtrip() {
        let t = TileId { z: 5, x: 40, y: 20 };
        let b = t.bounds();
        let c = TileId::covering(5, &Bounds { west: b.west + 1e-6, south: b.south + 1e-6, east: b.east - 1e-6, north: b.north - 1e-6 });
        assert_eq!(c, vec![t]);
    }
}
