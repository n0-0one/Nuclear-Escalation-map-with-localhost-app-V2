//! Road routing. Convoys in the event drive along the level's road splines
//! and go straight for the final approach, so a shortest path over the road
//! network is a good estimate of where a column travels.

use crate::geo::{Path, V2};
use crate::pack::RoadFile;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// Roads whose ends lie this close are treated as connected.
const JOIN_M: f64 = 50.0;
/// ...unless one passes over the other (flyovers).
const MAX_HEIGHT_GAP_M: f64 = 20.0;
/// A site farther than this from any road gets a straight-line route.
const MAX_ACCESS_M: f64 = 4000.0;

#[derive(Debug, Default)]
pub struct RoadGraph {
    nodes: Vec<V2>,
    edges: Vec<Vec<(u32, f32)>>,
    cache: HashMap<(i64, i64, i64, i64), Option<Path>>,
}

#[derive(PartialEq)]
struct Open(f64, u32);
impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl RoadGraph {
    pub fn build(file: &RoadFile) -> RoadGraph {
        let mut graph = RoadGraph::default();
        let mut heights: Vec<f64> = Vec::new();
        let mut road_of: Vec<u32> = Vec::new();
        for (road_index, road) in file.roads.iter().enumerate() {
            let mut previous: Option<u32> = None;
            for point in road.p.chunks_exact(3) {
                let index = graph.nodes.len() as u32;
                graph.nodes.push(V2(point[0], point[2]));
                graph.edges.push(Vec::new());
                heights.push(point[1]);
                road_of.push(road_index as u32);
                if let Some(prev) = previous {
                    let d = graph.nodes[prev as usize].dist(graph.nodes[index as usize]);
                    graph.link(prev, index, d);
                }
                previous = Some(index);
            }
        }
        // Separate splines often meet without sharing a point: join each node
        // to the nearest node of every other road within reach.
        let mut buckets: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for index in 0..graph.nodes.len() as u32 {
            let p = graph.nodes[index as usize];
            let (cx, cz) = ((p.0 / JOIN_M).floor() as i32, (p.1 / JOIN_M).floor() as i32);
            let mut nearest: HashMap<u32, (u32, f64)> = HashMap::new();
            for gx in cx - 1..=cx + 1 {
                for gz in cz - 1..=cz + 1 {
                    for &other in buckets.get(&(gx, gz)).into_iter().flatten() {
                        if road_of[other as usize] == road_of[index as usize] {
                            continue;
                        }
                        if (heights[other as usize] - heights[index as usize]).abs() > MAX_HEIGHT_GAP_M {
                            continue;
                        }
                        let d = graph.nodes[other as usize].dist(p);
                        if d > JOIN_M {
                            continue;
                        }
                        let entry = nearest.entry(road_of[other as usize]).or_insert((other, d));
                        if d < entry.1 {
                            *entry = (other, d);
                        }
                    }
                }
            }
            for (other, d) in nearest.into_values() {
                graph.link(index, other, d);
            }
            buckets.entry((cx, cz)).or_default().push(index);
        }
        if graph.nodes.is_empty() {
            if let Some(mesh) = &file.navmesh {
                graph.build_from_navmesh(&mesh.v, &mesh.t);
            }
        }
        graph
    }

    fn build_from_navmesh(&mut self, vertices: &[f64], triangles: &[u32]) {
        let vertex = |i: u32| V2(vertices[i as usize * 2], vertices[i as usize * 2 + 1]);
        let mut owners: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        for (index, tri) in triangles.chunks_exact(3).enumerate() {
            if tri.iter().any(|&i| (i as usize) * 2 + 1 >= vertices.len()) {
                continue;
            }
            let centre = vertex(tri[0]).add(vertex(tri[1])).add(vertex(tri[2])).scale(1.0 / 3.0);
            let node = self.nodes.len() as u32;
            self.nodes.push(centre);
            self.edges.push(Vec::new());
            let _ = index;
            for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                let key = (a.min(b), a.max(b));
                let list = owners.entry(key).or_default();
                for &other in list.iter() {
                    let d = self.nodes[other as usize].dist(centre);
                    self.link(other, node, d);
                }
                list.push(node);
            }
        }
    }

    fn link(&mut self, a: u32, b: u32, d: f64) {
        self.edges[a as usize].push((b, d as f32));
        self.edges[b as usize].push((a, d as f32));
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn nearest(&self, p: V2) -> Option<(u32, f64)> {
        let mut best: Option<(u32, f64)> = None;
        for (index, node) in self.nodes.iter().enumerate() {
            let d = node.dist(p);
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((index as u32, d));
            }
        }
        best
    }

    /// Route from `from` to `to`. Falls back to a straight line when there is
    /// no road network or the points are not connected by one.
    pub fn route(&mut self, from: V2, to: V2) -> Path {
        let key = (from.0.round() as i64, from.1.round() as i64, to.0.round() as i64, to.1.round() as i64);
        if !self.cache.contains_key(&key) {
            let found = self.search(from, to);
            self.cache.insert(key, found);
        }
        match &self.cache[&key] {
            Some(path) => path.clone(),
            None => Path::new(vec![from, to]),
        }
    }

    /// True when `route` would return a road path instead of a straight line.
    pub fn has_road_route(&mut self, from: V2, to: V2) -> bool {
        self.route(from, to).pts.len() > 2
    }

    fn search(&self, from: V2, to: V2) -> Option<Path> {
        let (start, start_gap) = self.nearest(from)?;
        let (goal, goal_gap) = self.nearest(to)?;
        if start_gap > MAX_ACCESS_M || goal_gap > MAX_ACCESS_M || start == goal {
            return None;
        }
        let count = self.nodes.len();
        let mut dist = vec![f64::INFINITY; count];
        let mut prev = vec![u32::MAX; count];
        let goal_point = self.nodes[goal as usize];
        let mut open = BinaryHeap::new();
        dist[start as usize] = 0.0;
        open.push(Open(self.nodes[start as usize].dist(goal_point), start));
        while let Some(Open(estimate, current)) = open.pop() {
            if current == goal {
                break;
            }
            let here = dist[current as usize];
            if estimate > here + self.nodes[current as usize].dist(goal_point) + 1e-3 {
                continue;
            }
            for &(next, step) in &self.edges[current as usize] {
                let candidate = here + step as f64;
                if candidate < dist[next as usize] {
                    dist[next as usize] = candidate;
                    prev[next as usize] = current;
                    open.push(Open(candidate + self.nodes[next as usize].dist(goal_point), next));
                }
            }
        }
        if !dist[goal as usize].is_finite() {
            return None;
        }
        let mut points = vec![to];
        let mut current = goal;
        loop {
            points.push(self.nodes[current as usize]);
            if current == start {
                break;
            }
            current = prev[current as usize];
            if current == u32::MAX {
                return None;
            }
        }
        points.push(from);
        points.reverse();
        // A detour of more than four times the direct distance means the
        // network does not really connect these places.
        let path = Path::new(points);
        if path.length() > from.dist(to) * 4.0 + 2000.0 {
            return None;
        }
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::Road;

    fn road(points: &[(f64, f64)]) -> Road {
        Road { s: "asphalt".into(), p: points.iter().flat_map(|&(x, z)| [x, 0.0, z]).collect() }
    }

    #[test]
    fn follows_joined_roads_instead_of_cutting_across() {
        let file = RoadFile {
            roads: vec![
                road(&[(0.0, 0.0), (1000.0, 0.0), (2000.0, 0.0)]),
                road(&[(2010.0, 5.0), (2010.0, 1000.0), (2010.0, 2000.0)]),
            ],
            navmesh: None,
        };
        let mut graph = RoadGraph::build(&file);
        let path = graph.route(V2(0.0, 10.0), V2(2000.0, 2000.0));
        assert!(path.pts.len() >= 6, "expected a road path, got {:?}", path.pts);
        assert!(path.length() > 3900.0 && path.length() < 4100.0);
        assert!(path.pts.iter().any(|p| p.dist(V2(2000.0, 0.0)) < 20.0), "must pass the corner");
    }

    #[test]
    fn falls_back_to_a_straight_line() {
        let mut empty = RoadGraph::build(&RoadFile::default());
        assert_eq!(empty.route(V2(0.0, 0.0), V2(5.0, 5.0)).pts.len(), 2);
        let file = RoadFile { roads: vec![road(&[(0.0, 0.0), (100.0, 0.0)]), road(&[(9000.0, 0.0), (9100.0, 0.0)])], navmesh: None };
        let mut split = RoadGraph::build(&file);
        assert_eq!(split.route(V2(0.0, 0.0), V2(9100.0, 0.0)).pts.len(), 2);
    }
}
