//! Small 2D helpers. World space is metres: x grows east, z grows north.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct V2(pub f64, pub f64);

impl V2 {
    pub fn x(self) -> f64 {
        self.0
    }
    pub fn z(self) -> f64 {
        self.1
    }
    pub fn sub(self, o: V2) -> V2 {
        V2(self.0 - o.0, self.1 - o.1)
    }
    pub fn add(self, o: V2) -> V2 {
        V2(self.0 + o.0, self.1 + o.1)
    }
    pub fn scale(self, k: f64) -> V2 {
        V2(self.0 * k, self.1 * k)
    }
    pub fn len(self) -> f64 {
        self.0.hypot(self.1)
    }
    pub fn dist(self, o: V2) -> f64 {
        self.sub(o).len()
    }
    pub fn dot(self, o: V2) -> f64 {
        self.0 * o.0 + self.1 * o.1
    }
    pub fn lerp(self, o: V2, t: f64) -> V2 {
        V2(self.0 + (o.0 - self.0) * t, self.1 + (o.1 - self.1) * t)
    }
    pub fn unit(self) -> V2 {
        let l = self.len();
        if l > 1e-9 {
            self.scale(1.0 / l)
        } else {
            V2(0.0, 0.0)
        }
    }
    /// Compass bearing in degrees: 0 = north, 90 = east.
    pub fn bearing(self) -> f64 {
        (self.0.atan2(self.1).to_degrees() + 360.0) % 360.0
    }
    pub fn round(self) -> V2 {
        V2(
            (self.0 * 10.0).round() / 10.0,
            (self.1 * 10.0).round() / 10.0,
        )
    }
}

/// Converts between the game's normalized map coordinates (0..1, y down)
/// and world metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapFrame {
    pub min: V2,
    pub max: V2,
}

impl MapFrame {
    pub fn from_bounds(b: [f64; 4]) -> Self {
        MapFrame {
            min: V2(b[0], b[1]),
            max: V2(b[2], b[3]),
        }
    }
    pub fn to_world(&self, nx: f64, ny: f64) -> V2 {
        V2(
            self.min.0 + nx * (self.max.0 - self.min.0),
            self.max.1 - ny * (self.max.1 - self.min.1),
        )
    }
    pub fn to_norm(&self, p: V2) -> (f64, f64) {
        (
            (p.0 - self.min.0) / (self.max.0 - self.min.0),
            (self.max.1 - p.1) / (self.max.1 - self.min.1),
        )
    }
    pub fn span(&self) -> f64 {
        (self.max.0 - self.min.0)
            .abs()
            .max((self.max.1 - self.min.1).abs())
    }
}

/// Uniform grid for "what is near this point" queries.
#[derive(Debug, Clone)]
pub struct Grid {
    cell: f64,
    buckets: HashMap<(i32, i32), Vec<u32>>,
    points: Vec<V2>,
}

impl Grid {
    pub fn new(cell: f64) -> Self {
        Grid {
            cell,
            buckets: HashMap::new(),
            points: Vec::new(),
        }
    }
    fn key(&self, p: V2) -> (i32, i32) {
        (
            (p.0 / self.cell).floor() as i32,
            (p.1 / self.cell).floor() as i32,
        )
    }
    pub fn insert(&mut self, p: V2) -> u32 {
        let index = self.points.len() as u32;
        self.points.push(p);
        let key = self.key(p);
        self.buckets.entry(key).or_default().push(index);
        index
    }
    pub fn len(&self) -> usize {
        self.points.len()
    }
    pub fn point(&self, index: u32) -> V2 {
        self.points[index as usize]
    }
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
    /// Calls `f(index, distance)` for every point within `radius` of `p`.
    pub fn each_near(&self, p: V2, radius: f64, mut f: impl FnMut(u32, f64)) {
        let reach = (radius / self.cell).ceil() as i32;
        let (cx, cz) = self.key(p);
        for gx in cx - reach..=cx + reach {
            for gz in cz - reach..=cz + reach {
                if let Some(bucket) = self.buckets.get(&(gx, gz)) {
                    for &index in bucket {
                        let d = self.points[index as usize].dist(p);
                        if d <= radius {
                            f(index, d);
                        }
                    }
                }
            }
        }
    }
    pub fn nearest(&self, p: V2, radius: f64) -> Option<(u32, f64)> {
        let mut best: Option<(u32, f64)> = None;
        self.each_near(p, radius, |index, d| {
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((index, d));
            }
        });
        best
    }
}

/// A polyline with cumulative lengths, for "where am I along this route".
#[derive(Debug, Clone, Default)]
pub struct Path {
    pub pts: Vec<V2>,
    pub cum: Vec<f64>,
}

impl Path {
    pub fn new(pts: Vec<V2>) -> Self {
        let mut cum = Vec::with_capacity(pts.len());
        let mut total = 0.0;
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                total += p.dist(pts[i - 1]);
            }
            cum.push(total);
        }
        Path { pts, cum }
    }
    pub fn length(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }
    /// Point and travel direction at distance `s` from the start.
    pub fn at(&self, s: f64) -> (V2, V2) {
        if self.pts.len() < 2 {
            return (self.pts.first().copied().unwrap_or_default(), V2(0.0, 0.0));
        }
        let s = s.clamp(0.0, self.length());
        let i = match self.cum.binary_search_by(|c| c.partial_cmp(&s).unwrap()) {
            Ok(i) => i.min(self.pts.len() - 2),
            Err(i) => i.saturating_sub(1).min(self.pts.len() - 2),
        };
        let seg = self.cum[i + 1] - self.cum[i];
        let t = if seg > 1e-9 {
            (s - self.cum[i]) / seg
        } else {
            0.0
        };
        (
            self.pts[i].lerp(self.pts[i + 1], t),
            self.pts[i + 1].sub(self.pts[i]).unit(),
        )
    }
    /// Closest point on the path: (distance along, distance off, direction).
    pub fn project(&self, p: V2) -> (f64, f64, V2) {
        let mut best = (0.0, f64::INFINITY, V2(0.0, 0.0));
        for i in 0..self.pts.len().saturating_sub(1) {
            let a = self.pts[i];
            let ab = self.pts[i + 1].sub(a);
            let l2 = ab.dot(ab);
            let t = if l2 > 1e-9 {
                (p.sub(a).dot(ab) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let d = a.add(ab.scale(t)).dist(p);
            if d < best.1 {
                best = (self.cum[i] + l2.sqrt() * t, d, ab.unit());
            }
        }
        best
    }
    /// The part of the path between two distances, as points.
    pub fn slice(&self, from: f64, to: f64) -> Vec<V2> {
        let (from, to) = (from.clamp(0.0, self.length()), to.clamp(0.0, self.length()));
        let mut out = vec![self.at(from).0];
        for (i, p) in self.pts.iter().enumerate() {
            if self.cum[i] > from && self.cum[i] < to {
                out.push(*p);
            }
        }
        out.push(self.at(to).0);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_frame_round_trips() {
        let frame = MapFrame::from_bounds([-65536.0, -65536.0, 65536.0, 65536.0]);
        let world = frame.to_world(0.25, 0.75);
        assert_eq!(world, V2(-32768.0, -32768.0));
        let (nx, ny) = frame.to_norm(world);
        assert!((nx - 0.25).abs() < 1e-12 && (ny - 0.75).abs() < 1e-12);
    }

    #[test]
    fn path_projection_and_sampling() {
        let path = Path::new(vec![V2(0.0, 0.0), V2(100.0, 0.0), V2(100.0, 100.0)]);
        assert_eq!(path.length(), 200.0);
        let (s, off, dir) = path.project(V2(50.0, 10.0));
        assert!((s - 50.0).abs() < 1e-9 && (off - 10.0).abs() < 1e-9 && dir == V2(1.0, 0.0));
        assert_eq!(path.at(150.0).0, V2(100.0, 50.0));
        assert_eq!(
            path.slice(50.0, 150.0),
            vec![V2(50.0, 0.0), V2(100.0, 0.0), V2(100.0, 50.0)]
        );
    }

    #[test]
    fn bearing_is_compass() {
        assert_eq!(V2(0.0, 1.0).bearing(), 0.0);
        assert_eq!(V2(1.0, 0.0).bearing(), 90.0);
        assert_eq!(V2(-1.0, 0.0).bearing(), 270.0);
    }
}
