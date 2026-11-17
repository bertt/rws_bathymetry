//! Data footprints of sparse rasters: polygons around the areas that contain valid cells.
//!
//! The raster is scanned at full resolution in blocks. Valid cells are counted per footprint cell
//! (`cell_px` x `cell_px` pixels); footprint cells that touch along an edge form one component,
//! whose outline (with holes) becomes a polygon in the raster's own coordinates.

use crate::source::Source;
use anyhow::Result;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use terrio::ops::Resampling;
use terrio::{FloatReadOptions, ResampledReadOptions, Window};

type Ring = Vec<(f64, f64)>;

/// A connected area with data. Each polygon is an outer ring followed by hole rings, in the source CRS (RD).
#[derive(Debug)]
pub struct Component {
    pub polygons: Vec<Vec<Ring>>,
    /// Number of raster cells (pixels) with a valid value.
    pub valid_cells: u64,
    /// Number of footprint cells that make up this component.
    pub footprint_cells: u64,
}

/// Number of valid pixels per footprint cell, keyed by (column, row) of the footprint grid.
fn count_cells(source: &Source, cell_px: usize) -> Result<HashMap<(i32, i32), u64>> {
    let probe = terrio::open_raster_lazy(&source.path.to_string_lossy())?;
    let (width, height) = (probe.width(), probe.height());
    drop(probe);
    let block = cell_px * (1024 / cell_px).max(1);
    let rows: Vec<usize> = (0..height).step_by(block).collect();
    let partial: Vec<HashMap<(i32, i32), u64>> = rows
        .par_iter()
        .map(|&y0| -> Result<HashMap<(i32, i32), u64>> {
            let mut r = terrio::open_raster_lazy(&source.path.to_string_lossy())?;
            let mut counts = HashMap::new();
            let h = block.min(height - y0);
            for x0 in (0..width).step_by(block) {
                let w = block.min(width - x0);
                let opts = ResampledReadOptions {
                    bands: vec![1],
                    window: Some((x0 as f64, y0 as f64, w as f64, h as f64)),
                    size: (w, h),
                    resampling: Resampling::Nearest,
                    ..Default::default()
                };
                let ds = r.read_resampled(&opts)?;
                let data = ds.read_window_f32(Window::full(w, h), &FloatReadOptions { bands: vec![1], ..Default::default() })?;
                for (i, v) in data.iter().enumerate() {
                    if v.is_finite() && v.abs() < 1.0e6 {
                        let (px, py) = (x0 + i % w, y0 + i / w);
                        *counts.entry(((px / cell_px) as i32, (py / cell_px) as i32)).or_insert(0) += 1;
                    }
                }
            }
            Ok(counts)
        })
        .collect::<Result<_>>()?;
    let mut all = HashMap::new();
    for p in partial {
        for (k, v) in p {
            *all.entry(k).or_insert(0) += v;
        }
    }
    Ok(all)
}

/// Footprint polygons of the valid data in `source`, one `Component` per connected area.
pub fn footprint(source: &Source, cell_px: usize) -> Result<Vec<Component>> {
    let counts = count_cells(source, cell_px)?;
    let r = terrio::open_raster_lazy(&source.path.to_string_lossy())?;
    let gt = r.geo_transform();
    let step = cell_px as f64;
    let to_rd = |(vx, vy): (i32, i32)| (gt[0] + f64::from(vx) * step * gt[1], gt[3] + f64::from(vy) * step * gt[5]);

    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let mut out = Vec::new();
    let mut keys: Vec<_> = counts.keys().copied().collect();
    keys.sort_unstable();
    for start in keys {
        if seen.contains(&start) {
            continue;
        }
        let mut cells = Vec::new();
        let mut queue = VecDeque::from([start]);
        seen.insert(start);
        while let Some(c) = queue.pop_front() {
            cells.push(c);
            for n in [(c.0 + 1, c.1), (c.0 - 1, c.1), (c.0, c.1 + 1), (c.0, c.1 - 1)] {
                if counts.contains_key(&n) && seen.insert(n) {
                    queue.push_back(n);
                }
            }
        }
        let set: HashSet<(i32, i32)> = cells.iter().copied().collect();
        let polygons = outline(&set)
            .into_iter()
            .map(|rings| rings.into_iter().map(|ring| ring.into_iter().map(to_rd).collect()).collect())
            .collect();
        out.push(Component {
            polygons,
            valid_cells: cells.iter().map(|c| counts[c]).sum(),
            footprint_cells: cells.len() as u64,
        });
    }
    Ok(out)
}

type V = (i32, i32);

/// Outline of a 4-connected set of grid cells as polygons (outer ring first, then holes), in grid-vertex coordinates.
fn outline(cells: &HashSet<V>) -> Vec<Vec<Vec<V>>> {
    // Directed boundary edges, clockwise around each cell in y-down coordinates (interior on the right).
    let mut edges: HashMap<V, Vec<V>> = HashMap::new();
    for &(i, j) in cells {
        if !cells.contains(&(i, j - 1)) {
            edges.entry((i, j)).or_default().push((i + 1, j));
        }
        if !cells.contains(&(i + 1, j)) {
            edges.entry((i + 1, j)).or_default().push((i + 1, j + 1));
        }
        if !cells.contains(&(i, j + 1)) {
            edges.entry((i + 1, j + 1)).or_default().push((i, j + 1));
        }
        if !cells.contains(&(i - 1, j)) {
            edges.entry((i, j + 1)).or_default().push((i, j));
        }
    }
    let mut rings: Vec<Vec<V>> = Vec::new();
    while let Some(&start) = edges.iter().find(|(_, v)| !v.is_empty()).map(|(k, _)| k) {
        let mut ring = vec![start];
        let mut cur = start;
        let mut dir: Option<V> = None;
        loop {
            let outs = edges.get_mut(&cur).expect("closed boundary");
            // Prefer the right turn so cells touching only at a corner stay separate.
            let pick = match dir {
                None => 0,
                Some((dx, dy)) => [(-dy, dx), (dx, dy), (dy, -dx)]
                    .iter()
                    .find_map(|d| outs.iter().position(|o| (o.0 - cur.0, o.1 - cur.1) == *d))
                    .unwrap_or(0),
            };
            let next = outs.swap_remove(pick);
            dir = Some((next.0 - cur.0, next.1 - cur.1));
            cur = next;
            if cur == start {
                break;
            }
            ring.push(cur);
        }
        rings.push(simplify(ring));
    }

    let (outers, holes): (Vec<_>, Vec<_>) = rings.into_iter().partition(|r| area(r) > 0.0);
    let mut polys: Vec<Vec<Vec<V>>> = outers.into_iter().map(|o| vec![o]).collect();
    for h in holes {
        // Probe point: middle of the hole's first edge, nudged so the ray never grazes a vertex.
        let p = (f64::from(h[0].0 + h[1].0) / 2.0, f64::from(h[0].1 + h[1].1) / 2.0 + 1e-3);
        let owner = polys.iter_mut().filter(|poly| contains(&poly[0], p)).min_by(|a, b| area(&a[0]).total_cmp(&area(&b[0])));
        if let Some(poly) = owner {
            poly.push(h);
        }
    }
    polys
}

fn area(r: &[V]) -> f64 {
    let n = r.len();
    (0..n).map(|k| f64::from(r[k].0) * f64::from(r[(k + 1) % n].1) - f64::from(r[(k + 1) % n].0) * f64::from(r[k].1)).sum::<f64>() / 2.0
}

fn contains(r: &[V], p: (f64, f64)) -> bool {
    let mut inside = false;
    let n = r.len();
    for k in 0..n {
        let (a, b) = ((f64::from(r[k].0), f64::from(r[k].1)), (f64::from(r[(k + 1) % n].0), f64::from(r[(k + 1) % n].1)));
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
    }
    inside
}

/// Drop vertices that lie on a straight line.
fn simplify(ring: Vec<V>) -> Vec<V> {
    let n = ring.len();
    (0..n)
        .filter(|&k| {
            let (p, c, q) = (ring[(k + n - 1) % n], ring[k], ring[(k + 1) % n]);
            (c.0 - p.0, c.1 - p.1) != (q.0 - c.0, q.1 - c.1)
        })
        .map(|k| ring[k])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_with_hole() {
        let mut cells = HashSet::new();
        for i in 0..3 {
            for j in 0..3 {
                if (i, j) != (1, 1) {
                    cells.insert((i, j));
                }
            }
        }
        let polys = outline(&cells);
        assert_eq!(polys.len(), 1);
        assert_eq!(polys[0].len(), 2);
        assert_eq!(polys[0][0].len(), 4);
        assert!((area(&polys[0][0]) - 9.0).abs() < 1e-9);
        assert!((area(&polys[0][1]) + 1.0).abs() < 1e-9);
    }

    #[test]
    fn single_cell() {
        let polys = outline(&HashSet::from([(5, 5)]));
        assert_eq!(polys.len(), 1);
        assert_eq!(polys[0].len(), 1);
        assert_eq!(polys[0][0].len(), 4);
        assert!((area(&polys[0][0]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn corner_touching_cells_stay_separate() {
        let polys = outline(&HashSet::from([(0, 0), (1, 1)]));
        assert_eq!(polys.len(), 2);
    }
}
