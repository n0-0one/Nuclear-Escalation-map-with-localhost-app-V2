//! Fixed units of the identified battle and what we know about each one.
//!
//! How a unit's state is decided, strongest evidence first:
//!
//! 1. A manual mark by the user.
//! 2. The unit is on the game's map right now -> alive.
//! 3. The kill feed names the unit type and one unit of that type just left
//!    the map (or only one exists) -> destroyed, confirmed.
//! 4. The unit left the map while its neighbours in the same site stayed
//!    visible -> destroyed, likely. Enemy units are only drawn while a
//!    friendly unit detects them, so vanishing alone proves nothing; the
//!    neighbours staying visible is what makes it meaningful.
//!
//! A unit seen again is alive again, whatever was decided before.

use crate::geo::{Grid, V2};
use crate::pack::{Level, Pack};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};

/// A map icon within this distance of a mission placement is that unit. The
/// game's icons can sit tens of metres off the mission position, and each
/// icon and each unit is used once per frame, nearest pairs first.
const MATCH_M: f64 = 60.0;
const MATCH_APPROX_M: f64 = 100.0;
/// An icon of the other team at a site counts as an occupier this much
/// worse than one of the site's own team at the same distance.
const FOREIGN_PENALTY_M: f64 = 25.0;
/// Occupiers must hold a site this long, with none of the owner's units in
/// sight there, before the site counts as captured.
const CAPTURE_HOLD_S: f64 = 4.0;
/// Units this close count as neighbours that share detection.
const WITNESS_M: f64 = 450.0;
/// A kill-feed line and a disappearance this close in time belong together.
const PAIR_S: f64 = 12.0;
/// Neighbours must stay visible this long after a unit vanishes before the
/// vanishing counts as a kill. When a detecting aircraft flies away, a whole
/// site drops off the map within a second or two of each other.
const WITNESS_HOLD_S: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Unknown,
    Alive,
    Destroyed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Proof {
    None,
    Likely,
    Confirmed,
    Manual,
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub site: u16,
    pub slot: u16,
    pub team: u8,
    pub pos: V2,
    pub id: String,
    pub approx: bool,
    pub restore_s: Option<f64>,
    pub status: Status,
    pub proof: Proof,
    pub seen_at: f64,
    /// Start of the current unbroken stretch of being visible.
    pub streak_from: f64,
    pub lost_at: Option<f64>,
    pub destroyed_at: Option<f64>,
    /// Index into `World::motions` for units that move (ships).
    pub mobile: Option<u16>,
}

/// How a ship moves: along its closed patrol route at a steady speed.
#[derive(Debug, Clone)]
pub struct Motion {
    pub path: crate::geo::Path,
    /// Metres per second.
    pub speed: f64,
    /// Position along the route at `t_ref`.
    pub s_ref: f64,
    pub t_ref: f64,
    /// The position comes from a sighting (rather than the battle clock or a guess).
    pub anchored: bool,
    /// Placed from the battle clock (true) or only guessed (false).
    pub timed: bool,
    last_obs: Option<(f64, f64)>,
}

impl Motion {
    pub fn s_at(&self, now: f64) -> f64 {
        let length = self.path.length().max(1.0);
        (self.s_ref + self.speed * (now - self.t_ref)).rem_euclid(length)
    }
    pub fn at(&self, now: f64) -> (V2, V2) {
        self.path.at(self.s_at(now))
    }
}

impl Unit {
    pub fn visible(&self, now: f64) -> bool {
        now - self.seen_at < 1.6
    }
    pub fn counts_as_alive(&self) -> bool {
        self.status != Status::Destroyed
    }
}

#[derive(Debug, Clone)]
struct PendingKill {
    ids: Vec<String>,
    at: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KillOutcome {
    /// Matched to one fixed unit: (site index, unit name).
    Located(u16, String),
    /// A fixed unit of that type died but we cannot tell which one yet.
    Unlocated(String),
    /// Not one of the fixed units (a convoy vehicle, for example).
    NotFixed,
}

pub struct World {
    pub level: usize,
    pub variant: usize,
    pub era: usize,
    pub my_team: u8,
    pub units: Vec<Unit>,
    pub site_units: Vec<Vec<u32>>,
    pub motions: Vec<Motion>,
    /// Current owner of each site, and who owned it at the start.
    pub owner: Vec<u8>,
    pub original: Vec<u8>,
    pub owner_proof: Vec<Proof>,
    capturable: Vec<bool>,
    occupied_since: Vec<Option<(u8, f64)>>,
    /// Same-team units close enough to share detection, per unit.
    neighbours: Vec<Vec<u32>>,
    grid: Grid,
    /// Grid entry -> unit index (moving units are not in the grid).
    grid_unit: Vec<u32>,
    /// How far the game's icons sit from the mission positions, learned from
    /// matched pairs (the same shift for the whole map).
    pub offset: V2,
    pending: VecDeque<PendingKill>,
    recent_vanish: VecDeque<(u32, f64)>,
    detect_loss_s: f64,
}

impl World {
    pub fn build(pack: &Pack, level: usize, variant: usize, era: usize, my_team: u8) -> World {
        let lvl: &Level = &pack.levels[level];
        let var = &lvl.variants[variant];
        let restore = &pack.core.template_restore;
        let mut units = Vec::new();
        let mut site_units = vec![Vec::new(); var.sites.len()];
        let mut grid = Grid::new(120.0);
        let mut grid_unit = Vec::new();
        let mut motions = Vec::new();
        for (si, site) in var.sites.iter().enumerate() {
            // Ships move along their routes; rocket artillery and missile
            // launchers wander inside their areas and are not tracked.
            let route = site.route.as_ref().filter(|r| site.cat == "ship" && r.pts.len() >= 4);
            if site.moves.is_some() && route.is_none() {
                continue;
            }
            for (ui, placement) in site.units.iter().enumerate() {
                if placement.e >> era & 1 == 0 {
                    continue;
                }
                let restore_s = match placement.l.as_deref() {
                    Some("restore") if restore.restore_seconds > 0.0 => Some(restore.restore_seconds),
                    Some("repair_with_site") if restore.depot_repair_seconds > 0.0 => Some(restore.depot_repair_seconds),
                    _ => None,
                };
                let mobile = route.map(|route| {
                    let mut points: Vec<V2> = route.pts.chunks_exact(2).map(|c| V2(c[0], c[1])).collect();
                    if route.closed {
                        points.push(points[0]);
                    }
                    // Ships are capped by their own top speed; the routes ask for more.
                    let kph = pack.core.units.get(&placement.u).and_then(|u| u.kph).unwrap_or(40.0).min(if route.speed_kph > 0.0 { route.speed_kph } else { 1e9 });
                    motions.push(Motion { path: crate::geo::Path::new(points), speed: kph / 3.6, s_ref: 0.0, t_ref: 0.0, anchored: false, timed: false, last_obs: None });
                    (motions.len() - 1) as u16
                });
                site_units[si].push(units.len() as u32);
                if mobile.is_none() {
                    grid.insert(placement.p);
                    grid_unit.push(units.len() as u32);
                }
                units.push(Unit {
                    site: si as u16,
                    slot: ui as u16,
                    team: site.team,
                    pos: placement.p,
                    id: placement.u.clone(),
                    approx: placement.x != 0,
                    restore_s,
                    status: Status::Unknown,
                    proof: Proof::None,
                    seen_at: f64::NEG_INFINITY,
                    streak_from: f64::NEG_INFINITY,
                    lost_at: None,
                    destroyed_at: None,
                    mobile,
                });
            }
        }
        let mut neighbours = vec![Vec::new(); units.len()];
        for (index, unit) in units.iter().enumerate() {
            if unit.mobile.is_some() {
                continue;
            }
            grid.each_near(unit.pos, WITNESS_M, |entry, _| {
                let other = grid_unit[entry as usize];
                if other as usize != index && units[other as usize].team == unit.team {
                    neighbours[index].push(other);
                }
            });
        }
        let original: Vec<u8> = var.sites.iter().map(|site| site.team).collect();
        let capturable = var.sites.iter().map(|site| matches!(site.cat.as_str(), "strongpoint" | "assembly_area" | "fuel_factory" | "ammo_factory" | "airfield")).collect();
        World {
            level,
            variant,
            era,
            my_team,
            units,
            site_units,
            motions,
            owner: original.clone(),
            owner_proof: vec![Proof::None; original.len()],
            occupied_since: vec![None; original.len()],
            original,
            capturable,
            neighbours,
            grid,
            grid_unit,
            offset: V2(0.0, 0.0),
            pending: VecDeque::new(),
            recent_vanish: VecDeque::new(),
            detect_loss_s: var.detect_loss_s,
        }
    }

    /// Pairs map icons with fixed units: nearest pairs first, each icon and
    /// each unit at most once. `icons` are positions with the icon's team when
    /// its colour tells. Returns, per icon, the unit and whether the icon
    /// belongs to the other team (an occupier at a capturable site).
    pub fn assign(&self, icons: &[(V2, Option<u8>)]) -> Vec<Option<(u32, bool)>> {
        let mut pairs: Vec<(f64, usize, u32, bool)> = Vec::new();
        for (ii, (raw, team)) in icons.iter().enumerate() {
            let position = &raw.sub(self.offset);
            self.grid.each_near(*position, MATCH_APPROX_M, |entry, d| {
                let index = self.grid_unit[entry as usize];
                let unit = &self.units[index as usize];
                if d > if unit.approx { MATCH_APPROX_M } else { MATCH_M } {
                    return;
                }
                let foreign = team.is_some_and(|t| t != unit.team);
                if foreign && !self.capturable[unit.site as usize] {
                    return;
                }
                pairs.push((d + if foreign { FOREIGN_PENALTY_M } else { 0.0 }, ii, index, foreign));
            });
        }
        pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut result = vec![None; icons.len()];
        let mut taken = vec![false; self.units.len()];
        for (_, ii, index, foreign) in pairs {
            if result[ii].is_some() || taken[index as usize] {
                continue;
            }
            taken[index as usize] = true;
            result[ii] = Some((index, foreign));
        }
        result
    }

    /// Learns the map-wide shift from this frame's matched (icon, unit) pairs.
    pub fn learn_offset(&mut self, pairs: &[(V2, u32)]) {
        if pairs.len() < 8 {
            return;
        }
        let mut dx: Vec<f64> = pairs.iter().map(|(p, i)| p.0 - self.units[*i as usize].pos.0).collect();
        let mut dz: Vec<f64> = pairs.iter().map(|(p, i)| p.1 - self.units[*i as usize].pos.1).collect();
        dx.sort_by(|a, b| a.partial_cmp(b).unwrap());
        dz.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = V2(dx[dx.len() / 2], dz[dz.len() / 2]);
        self.offset = self.offset.lerp(median, 0.3);
    }

    /// Kept for callers that match one icon at a time.
    pub fn match_icon(&self, position: V2, team: Option<u8>) -> Option<u32> {
        self.assign(&[(position, team)])[0].filter(|(_, foreign)| !foreign).map(|(index, _)| index)
    }

    /// Ships are placed on their routes from the battle clock until one is seen.
    pub fn place_ships(&mut self, battle_t: Option<f64>, now: f64) {
        for motion in &mut self.motions {
            if motion.anchored || (motion.timed && battle_t.is_some()) {
                continue;
            }
            match battle_t {
                Some(t) => {
                    motion.s_ref = (motion.speed * t).rem_euclid(motion.path.length().max(1.0));
                    motion.t_ref = now;
                    motion.timed = true;
                }
                None if motion.t_ref == 0.0 => motion.t_ref = now,
                None => {}
            }
        }
        for unit in &mut self.units {
            if let (Some(mi), true) = (unit.mobile, unit.status != Status::Destroyed) {
                unit.pos = self.motions[mi as usize].at(now).0;
            }
        }
    }

    /// Pairs leftover icons with ships. Returns, per icon, the ship's unit index.
    pub fn assign_ships(&self, icons: &[(V2, Option<u8>)], now: f64) -> Vec<Option<u32>> {
        let mut pairs: Vec<(f64, usize, u32)> = Vec::new();
        for (index, unit) in self.units.iter().enumerate() {
            let Some(mi) = unit.mobile else { continue };
            if unit.status == Status::Destroyed {
                continue;
            }
            let motion = &self.motions[mi as usize];
            let predicted = motion.at(now).0;
            let since = if unit.seen_at.is_finite() { now - unit.seen_at } else { f64::INFINITY };
            // Close to where it should be, or anywhere on its route while its
            // position is still only a guess.
            let gate = if motion.anchored { 1500.0 + 20.0 * since.min(600.0) } else { f64::INFINITY };
            for (ii, (position, team)) in icons.iter().enumerate() {
                if team.is_some_and(|t| t != unit.team) {
                    continue;
                }
                let off_route = motion.path.project(*position).1;
                if off_route > 900.0 {
                    continue;
                }
                let d = position.dist(predicted);
                if d <= gate {
                    pairs.push((if motion.anchored { d } else { off_route + d * 0.05 }, ii, index as u32));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut result = vec![None; icons.len()];
        let mut taken = vec![false; self.units.len()];
        for (_, ii, index) in pairs {
            if result[ii].is_none() && !taken[index as usize] {
                taken[index as usize] = true;
                result[ii] = Some(index);
            }
        }
        result
    }

    /// A ship was seen at `position`: fix its place on the route and learn its speed.
    pub fn see_ship(&mut self, index: u32, position: V2, now: f64) -> bool {
        if let Some(mi) = self.units[index as usize].mobile {
            let motion = &mut self.motions[mi as usize];
            let length = motion.path.length().max(1.0);
            let (s_obs, _, _) = motion.path.project(position);
            if let Some((s_prev, t_prev)) = motion.last_obs {
                let dt = now - t_prev;
                if dt >= 8.0 {
                    let ds = (s_obs - s_prev).rem_euclid(length);
                    let speed = ds / dt;
                    if (0.5..35.0).contains(&speed) {
                        motion.speed = motion.speed * 0.6 + speed * 0.4;
                    }
                    motion.last_obs = Some((s_obs, now));
                }
            } else {
                motion.last_obs = Some((s_obs, now));
            }
            motion.s_ref = s_obs;
            motion.t_ref = now;
            motion.anchored = true;
        }
        self.units[index as usize].pos = position;
        self.see(index, now)
    }

    /// Updates who holds each capturable site from this frame's occupiers:
    /// `occupiers[site] = Some((team, icons))`, `owner_seen[site]` = owner's
    /// units on the map there now. Returns sites that changed hands.
    pub fn update_owners(&mut self, occupiers: &HashMap<u16, (u8, u32)>, now: f64) -> Vec<(u16, u8)> {
        let mut changed = Vec::new();
        for site in 0..self.owner.len() {
            if !self.capturable[site] || self.owner_proof[site] == Proof::Manual {
                continue;
            }
            let owner_visible = self.site_units[site].iter().any(|&i| {
                let unit = &self.units[i as usize];
                unit.team == self.owner[site] && unit.visible(now)
            });
            let size = self.site_units[site].len() as u32;
            match occupiers.get(&(site as u16)) {
                Some(&(team, count)) if team != self.owner[site] && !owner_visible && count >= size.min(2).max(1) => {
                    let since = match self.occupied_since[site] {
                        Some((t, at)) if t == team => at,
                        _ => {
                            self.occupied_since[site] = Some((team, now));
                            now
                        }
                    };
                    if now - since >= CAPTURE_HOLD_S {
                        self.set_owner(site as u16, team, Proof::Confirmed, now);
                        changed.push((site as u16, team));
                    }
                }
                _ => self.occupied_since[site] = None,
            }
        }
        changed
    }

    /// Hands a site to `team`. Its garrison is now that team's units.
    pub fn set_owner(&mut self, site: u16, team: u8, proof: Proof, now: f64) {
        let si = site as usize;
        if si >= self.owner.len() {
            return;
        }
        let flipped = self.owner[si] != team;
        self.owner[si] = team;
        self.owner_proof[si] = if team == self.original[si] && proof == Proof::Manual { Proof::None } else { proof };
        self.occupied_since[si] = None;
        if flipped {
            for &index in &self.site_units[si] {
                let unit = &mut self.units[index as usize];
                unit.team = team;
                unit.status = if proof == Proof::Manual { Status::Unknown } else { Status::Alive };
                unit.proof = if proof == Proof::Manual { Proof::None } else { Proof::Confirmed };
                unit.destroyed_at = None;
                unit.lost_at = None;
                if proof != Proof::Manual {
                    unit.seen_at = now;
                    unit.streak_from = now;
                }
            }
        }
    }

    /// Records that a unit is on the map right now.
    pub fn see(&mut self, index: u32, now: f64) -> bool {
        let unit = &mut self.units[index as usize];
        if now - unit.seen_at > 1.6 {
            unit.streak_from = now;
        }
        unit.seen_at = now;
        unit.lost_at = None;
        let was_destroyed = unit.status == Status::Destroyed;
        unit.status = Status::Alive;
        unit.proof = Proof::Confirmed;
        unit.destroyed_at = None;
        was_destroyed
    }

    /// A neighbour that was visible before the unit vanished and still is.
    fn witnessed(&self, index: u32, lost_at: f64, now: f64) -> bool {
        self.neighbours[index as usize].iter().any(|&other| {
            let o = &self.units[other as usize];
            o.visible(now) && o.streak_from <= lost_at
        })
    }

    /// Call once per frame after all `see` calls. Returns units whose state
    /// changed to destroyed.
    pub fn settle(&mut self, now: f64) -> Vec<u32> {
        let mut changed = Vec::new();
        // newly vanished
        for index in 0..self.units.len() {
            let unit = &mut self.units[index];
            if unit.lost_at.is_none() && unit.seen_at.is_finite() && !unit.visible(now) && unit.status == Status::Alive {
                unit.lost_at = Some(unit.seen_at);
                self.recent_vanish.push_back((index as u32, unit.seen_at));
            }
        }
        while self.recent_vanish.front().is_some_and(|(_, at)| now - at > 120.0) {
            self.recent_vanish.pop_front();
        }
        while self.pending.front().is_some_and(|kill| now - kill.at > 900.0) {
            self.pending.pop_front();
        }

        // kill-feed lines still waiting for their icon to disappear
        let mut resolved = Vec::new();
        for (pi, kill) in self.pending.iter().enumerate() {
            if now - kill.at > PAIR_S + WITNESS_HOLD_S + 2.0 {
                continue;
            }
            if let Some(index) = self.vanished_candidate(&kill.ids, kill.at, now) {
                if !resolved.iter().any(|(_, taken)| *taken == index) {
                    resolved.push((pi, index));
                }
            }
        }
        for (pi, index) in resolved.into_iter().rev() {
            self.pending.remove(pi);
            self.destroy(index, Proof::Confirmed, now);
            changed.push(index);
        }

        // vanished while neighbours stayed visible
        let wait = self.detect_loss_s + 5.0;
        for index in 0..self.units.len() {
            let unit = &self.units[index];
            let Some(lost_at) = unit.lost_at else { continue };
            if unit.status != Status::Alive {
                continue;
            }
            let friendly = unit.team == self.my_team;
            // Friendly ground units are always sent, so a short absence is enough.
            let patience = match (friendly, unit.mobile.is_some()) {
                (true, true) => 8.0,
                (true, false) => 4.0,
                _ => wait,
            };
            if now - lost_at < patience {
                continue;
            }
            if !(friendly || self.witnessed(index as u32, lost_at, now)) {
                continue;
            }
            // A waiting kill-feed line for this unit type makes it certain.
            let id = unit.id.clone();
            let proof = match self.pending.iter().position(|kill| kill.ids.contains(&id) && (kill.at - lost_at).abs() < 120.0) {
                Some(pi) => {
                    self.pending.remove(pi);
                    Proof::Confirmed
                }
                None => Proof::Likely,
            };
            self.destroy(index as u32, proof, now);
            changed.push(index as u32);
        }

        // Process of elimination: of several units of the type a kill-feed
        // line named, all but one have been seen alive since. That one it is.
        let mut eliminated = Vec::new();
        for (pi, kill) in self.pending.iter().enumerate() {
            if now - kill.at < self.detect_loss_s + 5.0 {
                continue;
            }
            let candidates: Vec<u32> = (0..self.units.len() as u32)
                .filter(|&i| {
                    let unit = &self.units[i as usize];
                    kill.ids.contains(&unit.id) && unit.status != Status::Destroyed
                })
                .collect();
            let possible: Vec<u32> = candidates.iter().copied().filter(|&i| self.units[i as usize].seen_at < kill.at + 1.0).collect();
            if candidates.len() >= 2 && possible.len() == 1 && !eliminated.iter().any(|(_, i)| *i == possible[0]) {
                eliminated.push((pi, possible[0]));
            }
        }
        for (pi, index) in eliminated.into_iter().rev() {
            self.pending.remove(pi);
            self.destroy(index, Proof::Likely, now);
            changed.push(index);
        }
        changed
    }

    /// The unit a kill-feed line at `kill_at` most plausibly refers to, among
    /// units of the named type that have just left the map.
    fn vanished_candidate(&self, ids: &[String], kill_at: f64, now: f64) -> Option<u32> {
        let mut near: Vec<(u32, f64)> = self
            .recent_vanish
            .iter()
            .filter(|(index, at)| {
                let unit = &self.units[*index as usize];
                (kill_at - at).abs() < PAIR_S && ids.contains(&unit.id) && unit.status == Status::Alive && unit.lost_at.is_some()
            })
            .map(|(index, at)| (*index, *at))
            .collect();
        near.sort_by(|a, b| (kill_at - a.1).abs().partial_cmp(&(kill_at - b.1).abs()).unwrap());
        for &(index, at) in &near {
            let unit = &self.units[index as usize];
            // Friendly units are always on the map, so vanishing is enough.
            if unit.team == self.my_team && now - at >= 1.6 {
                return Some(index);
            }
            if now - at >= WITNESS_HOLD_S && self.witnessed(index, at, now) {
                return Some(index);
            }
        }
        // A unit with no neighbours (or a ship) can only be judged by timing.
        if let [(index, at)] = near[..] {
            let lone = self.neighbours[index as usize].is_empty() || self.units[index as usize].mobile.is_some();
            if lone && (kill_at - at).abs() < 4.0 && now - at >= WITNESS_HOLD_S {
                return Some(index);
            }
        }
        None
    }

    fn destroy(&mut self, index: u32, proof: Proof, now: f64) {
        let unit = &mut self.units[index as usize];
        unit.status = Status::Destroyed;
        unit.proof = proof;
        unit.destroyed_at = Some(unit.lost_at.unwrap_or(now));
        unit.lost_at = None;
    }

    /// Feeds one kill-feed line that names a unit type.
    pub fn kill_feed(&mut self, ids: &[String], now: f64, names: &HashMap<String, crate::pack::UnitSpec>) -> KillOutcome {
        let name = ids
            .first()
            .and_then(|id| names.get(id))
            .map_or_else(|| ids.first().cloned().unwrap_or_default(), |spec| spec.short().to_string());
        let candidates: Vec<u32> = (0..self.units.len() as u32)
            .filter(|&i| {
                let unit = &self.units[i as usize];
                ids.contains(&unit.id) && !(unit.status == Status::Destroyed && unit.proof != Proof::Likely)
            })
            .collect();
        if candidates.is_empty() {
            return KillOutcome::NotFixed;
        }
        // An earlier "likely" call for this type is now confirmed.
        if let Some(&index) = candidates.iter().find(|&&i| {
            let unit = &self.units[i as usize];
            unit.status == Status::Destroyed && unit.proof == Proof::Likely && unit.destroyed_at.is_some_and(|at| (now - at).abs() < 60.0)
        }) {
            self.units[index as usize].proof = Proof::Confirmed;
            return KillOutcome::Located(self.units[index as usize].site, name);
        }
        let alive: Vec<u32> = candidates.iter().copied().filter(|&i| self.units[i as usize].status != Status::Destroyed).collect();
        let pick = if alive.len() == 1 { Some(alive[0]) } else { self.vanished_candidate(ids, now, now) };
        match pick {
            Some(index) => {
                self.destroy(index, Proof::Confirmed, now);
                KillOutcome::Located(self.units[index as usize].site, name)
            }
            None => {
                self.pending.push_back(PendingKill { ids: ids.to_vec(), at: now });
                KillOutcome::Unlocated(name)
            }
        }
    }

    pub fn unlocated_kills(&self) -> usize {
        self.pending.len()
    }

    /// Manual mark for a whole site or one unit of it.
    pub fn mark(&mut self, site: u16, slot: Option<u16>, destroyed: Option<bool>, now: f64) {
        for unit in self.units.iter_mut().filter(|u| u.site == site && slot.map_or(true, |s| s == u.slot)) {
            match destroyed {
                Some(true) => {
                    unit.status = Status::Destroyed;
                    unit.proof = Proof::Manual;
                    unit.destroyed_at = Some(now);
                }
                Some(false) => {
                    unit.status = Status::Alive;
                    unit.proof = Proof::Manual;
                    unit.destroyed_at = None;
                }
                None => {
                    unit.status = Status::Unknown;
                    unit.proof = Proof::None;
                    unit.destroyed_at = None;
                }
            }
            unit.lost_at = None;
        }
    }

    /// Seconds until a destroyed unit may come back, if the mission restores it.
    pub fn restore_eta(&self, unit: &Unit, now: f64) -> Option<f64> {
        match (unit.status, unit.destroyed_at, unit.restore_s) {
            (Status::Destroyed, Some(at), Some(after)) => Some(at + after - now),
            _ => None,
        }
    }
}

/// Reads one kill-feed line. Returns the ids of the destroyed ground unit
/// type, or `None` when the line is not a ground kill we recognise.
pub fn parse_kill(text: &str, pack: &Pack) -> Option<Vec<String>> {
    let lower = text.to_lowercase();
    let hud = &pack.core.hud;
    let verbs = hud.verbs.get("ground").map(Vec::as_slice).unwrap_or(&[]);
    // "<killer> destroyed <victim>": the victim follows the verb.
    let mut victim_part: Option<&str> = None;
    for verb in verbs {
        if let Some(at) = lower.find(verb.as_str()) {
            victim_part = Some(&lower[at + verb.len()..]);
            break;
        }
    }
    let victim = victim_part?.trim().trim_matches(|c: char| c == '.' || c == '!' || c == ':').trim();
    if victim.is_empty() {
        return None;
    }
    if let Some(ids) = hud.names.get(victim) {
        return Some(ids.clone());
    }
    // Otherwise the longest known name contained in the victim part.
    hud.names
        .iter()
        .filter(|(name, _)| name.len() >= 3 && victim.contains(name.as_str()))
        .max_by_key(|(name, _)| name.len())
        .map(|(_, ids)| ids.clone())
}
