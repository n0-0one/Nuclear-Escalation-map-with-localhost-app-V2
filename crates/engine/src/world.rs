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

/// A map icon within this distance of a mission placement is that unit.
const MATCH_M: f64 = 30.0;
const MATCH_APPROX_M: f64 = 70.0;
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
    /// Same-team units close enough to share detection, per unit.
    neighbours: Vec<Vec<u32>>,
    grid: Grid,
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
        for (si, site) in var.sites.iter().enumerate() {
            if site.moves.is_some() {
                continue; // ships, MLRS and missile launchers do not stay put
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
                site_units[si].push(units.len() as u32);
                grid.insert(placement.p);
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
                });
            }
        }
        let mut neighbours = vec![Vec::new(); units.len()];
        for (index, unit) in units.iter().enumerate() {
            grid.each_near(unit.pos, WITNESS_M, |other, _| {
                if other as usize != index && units[other as usize].team == unit.team {
                    neighbours[index].push(other);
                }
            });
        }
        World {
            level,
            variant,
            era,
            my_team,
            units,
            site_units,
            neighbours,
            grid,
            pending: VecDeque::new(),
            recent_vanish: VecDeque::new(),
            detect_loss_s: var.detect_loss_s,
        }
    }

    /// Finds the fixed unit a map icon stands for. `team` is the icon's team
    /// when its colour tells us.
    pub fn match_icon(&self, position: V2, team: Option<u8>) -> Option<u32> {
        let mut best: Option<(u32, f64)> = None;
        self.grid.each_near(position, MATCH_APPROX_M, |index, d| {
            let unit = &self.units[index as usize];
            if team.is_some_and(|t| t != unit.team) {
                return;
            }
            if d > if unit.approx { MATCH_APPROX_M } else { MATCH_M } {
                return;
            }
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((index, d));
            }
        });
        best.map(|(index, _)| index)
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
            let patience = if friendly { 4.0 } else { wait };
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
        // A unit with no neighbours can only be judged by timing.
        if let [(index, at)] = near[..] {
            if self.neighbours[index as usize].is_empty() && (kill_at - at).abs() < 4.0 && now - at >= WITNESS_HOLD_S {
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
