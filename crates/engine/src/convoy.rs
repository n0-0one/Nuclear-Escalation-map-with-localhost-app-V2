//! Moving ground columns: following them while they are visible and
//! estimating where they are once they drop off the map.
//!
//! The event's own rules (read from the game script) make this tractable:
//! columns drive along roads at a fixed speed from a depot to one of its
//! three nearest destinations, and fuel/ammo depots send a column every time
//! their stock reaches a threshold, which works out to a fixed interval.

use crate::geo::{Path, V2};
use crate::pack::{ResourceRules, Variant};
use crate::roads::RoadGraph;
use serde::Serialize;

const TRACK_GATE_M: f64 = 45.0;
const TRACK_SPEED_GATE: f64 = 35.0;
const GROUP_M: f64 = 320.0;
const MOVED_M: f64 = 60.0;
const ROUTE_OFF_M: f64 = 220.0;
const DEFAULT_SPEED_FACTOR: f64 = 0.85;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteKind {
    Supply,
    Armor,
}

#[derive(Debug, Clone)]
pub struct Route {
    pub kind: RouteKind,
    pub team: u8,
    pub from: usize,
    pub to: usize,
    pub rank: u32,
    pub path: Path,
    /// Sites the route passes close to: (site index, distance along route, offset).
    pub passes: Vec<(usize, f64, f64)>,
}

#[derive(Debug, Clone)]
struct Track {
    team: u8,
    pos: V2,
    vel: V2,
    origin: V2,
    last: f64,
    hits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConvoyState {
    /// On the game's map right now.
    Observed,
    /// Was seen, now estimated along its route.
    Predicted,
    /// Never seen; expected from the depot's dispatch rhythm.
    Scheduled,
}

#[derive(Debug, Clone)]
pub struct Convoy {
    pub id: u32,
    pub team: u8,
    pub n: u32,
    pub pos: V2,
    pub dir: V2,
    pub speed: f64,
    pub state: ConvoyState,
    pub route: Option<usize>,
    /// Distance along the route at `last_seen`.
    pub s: f64,
    /// Current estimated distance along the route.
    pub s_now: f64,
    /// Half-length of the stretch of road the column is probably on.
    pub unc: f64,
    pub last_seen: f64,
    pub first_seen: f64,
    pub arrived_at: Option<f64>,
    updates: u32,
    last_fit: f64,
}

#[derive(Debug, Clone)]
pub struct Schedule {
    pub site: usize,
    pub team: u8,
    pub period: f64,
    /// Battle-time window of the first dispatch.
    pub window: (f64, f64),
    /// Battle time of one known dispatch, once a column has been seen.
    pub lock: Option<f64>,
    pub route: Option<usize>,
}

impl Schedule {
    /// Battle-time window of the next dispatch at or after `t`.
    pub fn next(&self, t: f64) -> (f64, f64) {
        let (lo, hi) = match self.lock {
            Some(lock) => (lock - 15.0, lock + 15.0),
            None => self.window,
        };
        let k = ((t - hi) / self.period).ceil().max(0.0);
        (lo + k * self.period, hi + k * self.period)
    }
}

#[derive(Default)]
pub struct Convoys {
    tracks: Vec<Track>,
    pub list: Vec<Convoy>,
    pub routes: Vec<Route>,
    pub schedules: Vec<Schedule>,
    next_id: u32,
    default_speed: f64,
}

/// One ground icon that is not a known fixed unit.
#[derive(Debug, Clone, Copy)]
pub struct Contact {
    pub pos: V2,
    /// 0 when the colour does not tell.
    pub team: u8,
}

impl Convoys {
    pub fn new() -> Self {
        Convoys { next_id: 1, default_speed: 17.0, ..Default::default() }
    }

    pub fn clear_live(&mut self) {
        self.tracks.clear();
        self.list.clear();
        for schedule in &mut self.schedules {
            schedule.lock = None;
        }
    }

    /// Works out every convoy route of the mission layout once.
    pub fn prepare(&mut self, variant: &Variant, graph: &mut RoadGraph, rules: &ResourceRules, pass_radius: f64) {
        self.routes.clear();
        self.schedules.clear();
        self.list.clear();
        self.tracks.clear();
        if rules.travel_speed > 0.0 {
            self.default_speed = rules.travel_speed * DEFAULT_SPEED_FACTOR;
        }
        let mut add = |kind: RouteKind, links: &[(String, String, u32)], routes: &mut Vec<Route>| {
            for (from_id, to_id, rank) in links {
                let (Some(from), Some(to)) = (variant.site_index(from_id), variant.site_index(to_id)) else { continue };
                let path = graph.route(variant.sites[from].p, variant.sites[to].p);
                let mut passes = Vec::new();
                for (si, site) in variant.sites.iter().enumerate() {
                    if site.moves.is_some() || si == from {
                        continue;
                    }
                    if !(site.is_depot() || site.cat == "strongpoint" || site.cat == "airfield") {
                        continue;
                    }
                    let (s, off, _) = path.project(site.p);
                    if off <= pass_radius {
                        passes.push((si, s, off));
                    }
                }
                routes.push(Route { kind, team: variant.sites[from].team, from, to, rank: *rank, path, passes });
            }
        };
        add(RouteKind::Supply, &variant.supply_links, &mut self.routes);
        add(RouteKind::Armor, &variant.armor_links, &mut self.routes);

        // Fuel and ammo depots dispatch on a fixed rhythm.
        if rules.production_per_second > 0.0 && rules.dispatch_threshold > 0.0 {
            let period = rules.dispatch_threshold / rules.production_per_second;
            let fraction = |i: usize, fallback: f64| rules.initial_fraction.get(i).copied().unwrap_or(fallback);
            let start = |f: f64| ((rules.dispatch_threshold - rules.capacity * f) / rules.production_per_second).max(0.0);
            let window = (start(fraction(1, 0.4)), start(fraction(0, 0.2)));
            let mut seen = Vec::new();
            for (ri, route) in self.routes.iter().enumerate() {
                if route.kind != RouteKind::Supply || seen.contains(&route.from) {
                    continue;
                }
                seen.push(route.from);
                let nearest = self
                    .routes
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.kind == RouteKind::Supply && r.from == route.from)
                    .min_by_key(|(_, r)| r.rank)
                    .map(|(i, _)| i)
                    .unwrap_or(ri);
                self.schedules.push(Schedule { site: route.from, team: route.team, period, window, lock: None, route: Some(nearest) });
            }
        }
    }

    fn effective_speed(&self, convoy: &Convoy) -> f64 {
        if convoy.speed > 3.0 {
            convoy.speed.clamp(5.0, 25.0)
        } else {
            self.default_speed
        }
    }

    /// Advances tracking with this frame's unmatched ground icons.
    pub fn update(&mut self, now: f64, contacts: &[Contact], fresh: bool, battle_t: Option<f64>) {
        if fresh {
            self.update_tracks(now, contacts);
            self.update_groups(now);
        }
        self.advance(now);
        if let Some(t) = battle_t {
            self.update_schedules(t);
        }
    }

    fn update_tracks(&mut self, now: f64, contacts: &[Contact]) {
        let mut used = vec![false; self.tracks.len()];
        for contact in contacts {
            let mut best: Option<(usize, f64)> = None;
            for (ti, track) in self.tracks.iter().enumerate() {
                if used[ti] || (track.team != 0 && contact.team != 0 && track.team != contact.team) {
                    continue;
                }
                let dt = (now - track.last).max(0.05);
                let predicted = track.pos.add(track.vel.scale(dt));
                let d = predicted.dist(contact.pos).min(track.pos.dist(contact.pos));
                if d <= TRACK_GATE_M + TRACK_SPEED_GATE * dt && best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((ti, d));
                }
            }
            match best {
                Some((ti, _)) => {
                    used[ti] = true;
                    let track = &mut self.tracks[ti];
                    let dt = now - track.last;
                    if dt > 0.15 {
                        let velocity = contact.pos.sub(track.pos).scale(1.0 / dt);
                        if velocity.len() < 60.0 {
                            track.vel = track.vel.lerp(velocity, 0.35);
                        }
                        track.pos = contact.pos;
                        track.last = now;
                        track.hits += 1;
                    }
                    if contact.team != 0 {
                        track.team = contact.team;
                    }
                }
                None => {
                    self.tracks.push(Track { team: contact.team, pos: contact.pos, vel: V2(0.0, 0.0), origin: contact.pos, last: now, hits: 1 });
                    used.push(true);
                }
            }
        }
        self.tracks.retain(|track| now - track.last < 4.0);
        if self.tracks.len() > 1500 {
            self.tracks.truncate(1500);
        }
    }

    fn update_groups(&mut self, now: f64) {
        // Vehicles that have actually travelled, grouped by proximity.
        let moving: Vec<usize> = (0..self.tracks.len())
            .filter(|&i| {
                let t = &self.tracks[i];
                t.team != 0 && now - t.last < 1.6 && t.hits >= 3 && t.pos.dist(t.origin) > MOVED_M
            })
            .collect();
        let mut group = vec![usize::MAX; moving.len()];
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for a in 0..moving.len() {
            if group[a] != usize::MAX {
                continue;
            }
            let gi = groups.len();
            groups.push(vec![moving[a]]);
            group[a] = gi;
            let mut queue = vec![a];
            while let Some(current) = queue.pop() {
                for b in 0..moving.len() {
                    if group[b] == usize::MAX
                        && self.tracks[moving[b]].team == self.tracks[moving[current]].team
                        && self.tracks[moving[b]].pos.dist(self.tracks[moving[current]].pos) < GROUP_M
                    {
                        group[b] = gi;
                        groups[gi].push(moving[b]);
                        queue.push(b);
                    }
                }
            }
        }

        let mut claimed = vec![false; self.list.len()];
        for members in groups {
            let n = members.len() as u32;
            let team = self.tracks[members[0]].team;
            let centre = members.iter().fold(V2(0.0, 0.0), |acc, &i| acc.add(self.tracks[i].pos)).scale(1.0 / n as f64);
            let velocity = members.iter().fold(V2(0.0, 0.0), |acc, &i| acc.add(self.tracks[i].vel)).scale(1.0 / n as f64);

            let mut best: Option<(usize, f64)> = None;
            for (ci, convoy) in self.list.iter().enumerate() {
                if claimed[ci] || convoy.team != team || convoy.state == ConvoyState::Scheduled {
                    continue;
                }
                let gate = 450.0 + convoy.unc + 30.0 * (now - convoy.last_seen).min(60.0);
                let d = convoy.pos.dist(centre);
                if d < gate && best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((ci, d));
                }
            }
            let ci = match best {
                Some((ci, _)) => ci,
                None => {
                    if n < 2 && !self.on_some_route(centre, team) {
                        continue; // a lone vehicle off the roads is not a column
                    }
                    let id = self.next_id;
                    self.next_id += 1;
                    self.list.push(Convoy {
                        id,
                        team,
                        n,
                        pos: centre,
                        dir: velocity.unit(),
                        speed: velocity.len(),
                        state: ConvoyState::Observed,
                        route: None,
                        s: 0.0,
                        s_now: 0.0,
                        unc: 0.0,
                        last_seen: now,
                        first_seen: now,
                        arrived_at: None,
                        updates: 0,
                        last_fit: f64::NEG_INFINITY,
                    });
                    claimed.push(false);
                    self.list.len() - 1
                }
            };
            claimed[ci] = true;
            let convoy = &mut self.list[ci];
            let was_recent = now - convoy.last_seen < 12.0;
            convoy.n = if was_recent { convoy.n.max(n) } else { n };
            convoy.pos = centre;
            if velocity.len() > 1.0 {
                convoy.dir = velocity.unit();
                convoy.speed = if convoy.updates > 2 { convoy.speed * 0.8 + velocity.len() * 0.2 } else { velocity.len() };
            }
            convoy.state = ConvoyState::Observed;
            convoy.last_seen = now;
            convoy.unc = 0.0;
            convoy.arrived_at = None;
            convoy.updates += 1;
            if convoy.updates >= 3 && now - convoy.last_fit > 4.0 {
                self.fit_route(ci, now);
            }
            if let Some(ri) = self.list[ci].route {
                let (s, _, _) = self.routes[ri].path.project(centre);
                self.list[ci].s = s;
                self.list[ci].s_now = s;
            }
        }
    }

    fn on_some_route(&self, p: V2, team: u8) -> bool {
        self.routes.iter().any(|route| route.team == team && route.path.project(p).1 < ROUTE_OFF_M * 0.6)
    }

    fn fit_route(&mut self, ci: usize, now: f64) {
        let convoy = &self.list[ci];
        let mut best: Option<(usize, f64)> = None;
        for (ri, route) in self.routes.iter().enumerate() {
            if route.team != convoy.team {
                continue;
            }
            let (s, off, dir) = route.path.project(convoy.pos);
            if off > ROUTE_OFF_M {
                continue;
            }
            let agreement = if convoy.speed > 2.0 { convoy.dir.dot(dir) } else { 1.0 };
            if agreement < 0.25 {
                continue;
            }
            // Prefer the route we already follow so shared stretches do not flip-flop.
            let keep = if convoy.route == Some(ri) { -80.0 } else { 0.0 };
            let done = if route.path.length() - s < 30.0 { 500.0 } else { 0.0 };
            let cost = off + (1.0 - agreement) * 250.0 + route.rank as f64 * 25.0 + keep + done;
            if best.map_or(true, |(_, bc)| cost < bc) {
                best = Some((ri, cost));
            }
        }
        let convoy = &mut self.list[ci];
        convoy.last_fit = now;
        convoy.route = best.map(|(ri, _)| ri);
    }

    fn advance(&mut self, now: f64) {
        let default_speed = self.default_speed;
        for ci in 0..self.list.len() {
            let speed = self.effective_speed(&self.list[ci]);
            let convoy = &mut self.list[ci];
            if convoy.state == ConvoyState::Scheduled {
                continue;
            }
            let gap = now - convoy.last_seen;
            if gap < 1.6 {
                continue;
            }
            convoy.state = ConvoyState::Predicted;
            match convoy.route {
                Some(ri) => {
                    let path = &self.routes[ri].path;
                    convoy.s_now = convoy.s + speed * gap;
                    convoy.unc = 80.0 + 0.25 * speed * gap;
                    let (pos, dir) = path.at(convoy.s_now);
                    convoy.pos = pos;
                    convoy.dir = dir;
                    if convoy.s_now >= path.length() && convoy.arrived_at.is_none() {
                        convoy.arrived_at = Some(now);
                    }
                }
                None => {
                    // No road to follow: coast in a straight line for a short while.
                    let step = if convoy.speed > 1.0 { convoy.speed } else { default_speed };
                    convoy.unc = 120.0 + 0.5 * step * gap;
                    if gap > 45.0 && convoy.arrived_at.is_none() {
                        convoy.arrived_at = Some(now - 30.0);
                    }
                }
            }
        }
        self.list.retain(|convoy| {
            convoy.n > 0 && convoy.arrived_at.map_or(true, |at| now - at < 20.0) && now - convoy.last_seen < 1500.0
        });
    }

    fn update_schedules(&mut self, battle_t: f64) {
        // A seen supply column tells us when its depot dispatched it.
        for convoy in &self.list {
            if convoy.state != ConvoyState::Observed || convoy.updates < 4 {
                continue;
            }
            let Some(ri) = convoy.route else { continue };
            let route = &self.routes[ri];
            if route.kind != RouteKind::Supply || convoy.s > route.path.length() * 0.9 {
                continue;
            }
            let speed = if convoy.speed > 3.0 { convoy.speed.clamp(5.0, 25.0) } else { self.default_speed };
            let dispatched = battle_t - convoy.s / speed;
            if let Some(schedule) = self.schedules.iter_mut().find(|s| s.site == route.from) {
                schedule.lock = Some(dispatched);
                schedule.route = Some(ri);
            }
        }
    }

    /// Columns expected to be on the road right now from locked schedules
    /// that no tracked convoy accounts for.
    pub fn scheduled(&self, battle_t: f64, enemy_team: u8) -> Vec<Convoy> {
        let mut out = Vec::new();
        for schedule in &self.schedules {
            let (Some(lock), Some(ri)) = (schedule.lock, schedule.route) else { continue };
            if schedule.team != enemy_team {
                continue;
            }
            let route = &self.routes[ri];
            let travel = route.path.length() / self.default_speed;
            let mut k = ((battle_t - travel - lock) / schedule.period).ceil();
            while lock + k * schedule.period <= battle_t {
                let dispatched = lock + k * schedule.period;
                let s = (battle_t - dispatched) * self.default_speed;
                let covered = self.list.iter().any(|c| {
                    c.route.is_some_and(|r| self.routes[r].from == schedule.site) && (c.s_now - s).abs() < 2500.0
                });
                if !covered && k != 0.0 {
                    let (pos, dir) = route.path.at(s);
                    out.push(Convoy {
                        id: 900_000 + schedule.site as u32 * 100 + (k.rem_euclid(100.0)) as u32,
                        team: schedule.team,
                        n: 5,
                        pos,
                        dir,
                        speed: self.default_speed,
                        state: ConvoyState::Scheduled,
                        route: Some(ri),
                        s,
                        s_now: s,
                        unc: 400.0 + 0.1 * s,
                        last_seen: f64::NEG_INFINITY,
                        first_seen: f64::NEG_INFINITY,
                        arrived_at: None,
                        updates: 0,
                        last_fit: 0.0,
                    });
                }
                k += 1.0;
            }
        }
        out
    }

    /// A kill-feed line named a vehicle that is not a fixed unit: take one
    /// vehicle off the column of that team seen most recently.
    pub fn kill(&mut self, team: u8, now: f64) -> Option<u32> {
        let convoy = self
            .list
            .iter_mut()
            .filter(|c| c.team == team && c.state != ConvoyState::Scheduled && now - c.last_seen < 40.0)
            .max_by(|a, b| a.last_seen.partial_cmp(&b.last_seen).unwrap())?;
        convoy.n = convoy.n.saturating_sub(1);
        Some(convoy.id)
    }

    /// Seconds until the convoy reaches distance `s` on its route.
    pub fn eta(&self, convoy: &Convoy, s: f64) -> f64 {
        (s - convoy.s_now) / self.effective_speed(convoy)
    }
}
