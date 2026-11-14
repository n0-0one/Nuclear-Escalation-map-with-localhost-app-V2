//! Works out which battle the player is in: map, which half of it is
//! friendly, which side layout is in use, which team the player is on and
//! which BR bracket - using only what the game shows on its map.
//!
//! Several independent clues are combined, so that no single one failing
//! leaves the app without an answer:
//!
//! * Map: runway positions.
//! * Friendly half: the colour of airfields, which half the friendly and
//!   enemy ground icons sit in (works even if positions are a bit off), and
//!   where the player first appears.
//! * Team: the nation of the player's vehicle (USSR and China fly for team 1,
//!   everyone else for team 2), and the exact pattern of ground units, which
//!   differs between the two factions.
//! * BR bracket: the pattern of ground units (every bracket fields different
//!   air-defence systems), unit names in the kill feed and the battle rating
//!   of the player's own vehicle (a battle cannot be in a bracket below it).
//!
//! The friendly half plus the team give the layout: in the swapped mission
//! team 1 sits where team 2 normally does.

use crate::geo::{Grid, MapFrame, V2};
use crate::pack::Pack;
use crate::telemetry::Aff;
use serde::Serialize;
use std::collections::HashMap;

/// A ground icon must be seen this many times in one spot to count as fixed.
const STATIC_HITS: u16 = 4;
const CELL_M: f64 = 8.0;
const MATCH_M: f64 = 30.0;
/// Used when exact positions do not line up: "somewhere in that site".
const COARSE_MATCH_M: f64 = 160.0;
const RUNWAY_MATCH_M: f64 = 700.0;
/// How much the vehicle's nation counts, in units of matching ground units.
const NATION_BONUS: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    None,
    Guess,
    Likely,
    Confirmed,
    Manual,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Manual {
    pub level: Option<String>,
    pub variant: Option<String>,
    pub team: Option<u8>,
    pub era: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Identity {
    pub level: Option<usize>,
    pub variant: Option<usize>,
    pub team: Option<u8>,
    pub era: Option<usize>,
    pub level_conf: Confidence,
    pub side_conf: Confidence,
    pub era_conf: Confidence,
    /// Plain-language reasons, shown to the user.
    pub why: Vec<String>,
    /// Brackets that fit the evidence equally well.
    pub era_options: Vec<usize>,
}

impl Default for Identity {
    fn default() -> Self {
        Identity {
            level: None,
            variant: None,
            team: None,
            era: None,
            level_conf: Confidence::None,
            side_conf: Confidence::None,
            era_conf: Confidence::None,
            why: Vec::new(),
            era_options: Vec::new(),
        }
    }
}

impl Identity {
    pub fn complete(&self) -> bool {
        self.level.is_some() && self.variant.is_some() && self.team.is_some() && self.era.is_some()
    }
}

/// What the detection saw, for the "copy diagnostics" button.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Diagnostics {
    pub airfields_friendly: usize,
    pub airfields_enemy: usize,
    pub airfields_other: usize,
    pub runways_matched: usize,
    pub fixed_friendly: usize,
    pub fixed_enemy: usize,
    pub fixed_other: usize,
    /// Fixed ground icons within 30 m / 160 m of a mission placement.
    pub matched_exact: u32,
    pub matched_coarse: u32,
    pub half_votes: (u32, u32),
    pub nation: String,
    pub nation_team: Option<u8>,
    pub used_coarse: bool,
}

/// Pre-indexed placements of one level, built once per pack.
struct LevelIndex {
    /// (variant, team, era mask) for each indexed point.
    meta: Vec<(u8, u8, u32)>,
    grid: Grid,
    /// Runway midpoints with the team that owns them in the first variant.
    runways: Vec<(V2, u8)>,
    /// Site positions of team 1 and team 2 in the first (standard) layout.
    half_a: Vec<V2>,
    half_b: Vec<V2>,
}

fn nearest(points: &[V2], p: V2) -> f64 {
    points.iter().map(|q| q.dist(p)).fold(f64::INFINITY, f64::min)
}

pub struct Identifier {
    index: Vec<LevelIndex>,
    airfields: Vec<(V2, Aff)>,
    cells: HashMap<(i32, i32), (V2, Aff, u16)>,
    hud_units: HashMap<String, u32>,
    first_player: Option<V2>,
    dirty: bool,
    pub identity: Identity,
    pub diagnostics: Diagnostics,
}

impl Identifier {
    pub fn new(pack: &Pack) -> Self {
        let index = pack
            .levels
            .iter()
            .map(|level| {
                let mut grid = Grid::new(100.0);
                let mut meta = Vec::new();
                for (vi, variant) in level.variants.iter().enumerate() {
                    for site in &variant.sites {
                        if site.moves.is_some() {
                            continue;
                        }
                        for unit in &site.units {
                            grid.insert(unit.p);
                            meta.push((vi as u8, site.team, unit.e));
                        }
                    }
                }
                let standard = &level.variants[0];
                let runways = standard
                    .sites
                    .iter()
                    .flat_map(|site| site.rw.iter().map(move |r| (V2((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0), site.team)))
                    .collect();
                let half = |team: u8| standard.sites.iter().filter(|s| s.team == team && s.moves.is_none()).map(|s| s.p).collect();
                LevelIndex { meta, grid, runways, half_a: half(1), half_b: half(2) }
            })
            .collect();
        Identifier {
            index,
            airfields: Vec::new(),
            cells: HashMap::new(),
            hud_units: HashMap::new(),
            first_player: None,
            dirty: true,
            identity: Identity::default(),
            diagnostics: Diagnostics::default(),
        }
    }

    pub fn reset(&mut self) {
        self.airfields.clear();
        self.cells.clear();
        self.hud_units.clear();
        self.first_player = None;
        self.dirty = true;
        self.identity = Identity::default();
        self.diagnostics = Diagnostics::default();
    }

    pub fn see_airfields(&mut self, airfields: Vec<(V2, Aff)>) {
        if airfields.len() != self.airfields.len()
            || airfields.iter().zip(&self.airfields).any(|(a, b)| a.1 != b.1 || a.0.dist(b.0) > 50.0)
        {
            self.airfields = airfields;
            self.dirty = true;
        }
    }

    pub fn see_ground(&mut self, position: V2, aff: Aff) {
        if self.cells.len() >= 6000 {
            return;
        }
        let key = ((position.0 / CELL_M).floor() as i32, (position.1 / CELL_M).floor() as i32);
        let cell = self.cells.entry(key).or_insert((position, aff, 0));
        cell.1 = aff;
        cell.2 = cell.2.saturating_add(1);
        if cell.2 == STATIC_HITS {
            self.dirty = true;
        }
    }

    /// Where the player is; only the first position of a battle is kept.
    pub fn see_player(&mut self, position: V2) {
        if self.first_player.is_none() {
            self.first_player = Some(position);
            self.dirty = true;
        }
    }

    pub fn see_kill(&mut self, unit_ids: &[String]) {
        for id in unit_ids {
            *self.hud_units.entry(id.clone()).or_insert(0) += 1;
        }
        self.dirty = true;
    }

    /// Fingerprint score per [variant][my team - 1][era] at a match radius,
    /// plus how many fixed icons matched anything.
    fn fingerprint(&self, index: &LevelIndex, variants: usize, eras: usize, radius: f64, penalise_missing: bool) -> (Vec<[Vec<u32>; 2]>, u32) {
        let mut score = vec![[vec![0u32; eras], vec![0u32; eras]]; variants];
        let mut matched = 0u32;
        for (position, aff, hits) in self.cells.values() {
            if *hits < STATIC_HITS {
                continue;
            }
            // era masks of matching placements per (variant, unit team)
            let mut found = vec![[0u32; 2]; variants];
            index.grid.each_near(*position, radius, |i, _| {
                let (vi, team, mask) = index.meta[i as usize];
                if (1..=2).contains(&team) {
                    found[vi as usize][team as usize - 1] |= mask;
                }
            });
            if found.iter().any(|f| f[0] | f[1] != 0) {
                matched += 1;
            }
            for vi in 0..variants {
                for my in 0..2usize {
                    let mask = match aff {
                        Aff::Ally | Aff::Player => found[vi][my],
                        Aff::Enemy => found[vi][1 - my],
                        Aff::Unknown => found[vi][0] | found[vi][1],
                    };
                    for (ei, slot) in score[vi][my].iter_mut().enumerate() {
                        if mask >> ei & 1 == 1 {
                            *slot += 1;
                        }
                    }
                }
            }
        }
        if !penalise_missing {
            return (score, matched);
        }
        // Friendly ground units are always on the map, so a layout that
        // expects friendly units where none has ever been seen loses points.
        let mut seen = Grid::new(100.0);
        let mut friendly_seen = 0usize;
        for (position, aff, hits) in self.cells.values() {
            if *hits >= STATIC_HITS && matches!(aff, Aff::Ally | Aff::Player) {
                seen.insert(*position);
                friendly_seen += 1;
            }
        }
        if friendly_seen >= 15 {
            for (i, &(vi, team, mask)) in index.meta.iter().enumerate() {
                if !(1..=2).contains(&team) || seen.nearest(index.grid.point(i as u32), radius).is_some() {
                    continue;
                }
                for (ei, slot) in score[vi as usize][team as usize - 1].iter_mut().enumerate() {
                    if mask >> ei & 1 == 1 {
                        *slot = slot.saturating_sub(1);
                    }
                }
            }
        }
        (score, matched)
    }

    /// Re-evaluates when new evidence arrived. Returns true if the identity
    /// (not just its confidence) changed. `vehicle` is the player's vehicle
    /// as (economic rank, country).
    pub fn evaluate(&mut self, pack: &Pack, frame: Option<MapFrame>, vehicle: Option<(i32, &str)>, manual: &Manual) -> bool {
        if !self.dirty {
            return false;
        }
        self.dirty = false;
        let mut next = Identity::default();
        let mut diag = Diagnostics::default();
        let eras = &pack.core.eras;
        let vehicle_rank = vehicle.map(|v| v.0);
        for (_, aff) in &self.airfields {
            match aff {
                Aff::Ally | Aff::Player => diag.airfields_friendly += 1,
                Aff::Enemy => diag.airfields_enemy += 1,
                Aff::Unknown => diag.airfields_other += 1,
            }
        }
        for (_, aff, hits) in self.cells.values() {
            if *hits >= STATIC_HITS {
                match aff {
                    Aff::Ally | Aff::Player => diag.fixed_friendly += 1,
                    Aff::Enemy => diag.fixed_enemy += 1,
                    Aff::Unknown => diag.fixed_other += 1,
                }
            }
        }
        if let Some((_, country)) = vehicle {
            diag.nation = country.to_string();
            diag.nation_team = match country {
                "" => None,
                "ussr" | "china" => Some(1),
                _ => Some(2),
            };
        }

        // ---- map
        let manual_level = manual.level.as_ref().and_then(|id| pack.levels.iter().position(|l| &l.id == id));
        let mut level_scores: Vec<(usize, usize)> = Vec::new();
        for (li, level) in pack.levels.iter().enumerate() {
            if let Some(frame) = frame {
                let level_frame = MapFrame::from_bounds(level.bounds);
                if (frame.span() - level_frame.span()).abs() > level_frame.span() * 0.02 {
                    continue;
                }
            }
            let hits = self
                .airfields
                .iter()
                .filter(|(p, _)| self.index[li].runways.iter().any(|(r, _)| r.dist(*p) < RUNWAY_MATCH_M))
                .count();
            level_scores.push((li, hits));
        }
        level_scores.sort_by(|a, b| b.1.cmp(&a.1));
        if let Some(li) = manual_level {
            next.level = Some(li);
            next.level_conf = Confidence::Manual;
            next.why.push("Map chosen by you.".into());
        } else if let Some(&(li, hits)) = level_scores.first() {
            let second = level_scores.get(1).map_or(0, |s| s.1);
            if hits >= 2 && hits > second {
                next.level = Some(li);
                next.level_conf = if hits >= 3 { Confidence::Confirmed } else { Confidence::Likely };
                next.why.push(format!("Map: {hits} runways match {}.", pack.levels[li].name));
            } else if level_scores.len() == 1 && frame.is_some() && self.airfields.len() < 2 {
                // Same size and no runways seen yet. Once runways are visible
                // and none fits, this is a map the data pack does not know.
                next.level = Some(li);
                next.level_conf = Confidence::Guess;
                next.why.push(format!("Map: only {} has this size.", pack.levels[li].name));
            }
            diag.runways_matched = hits;
        }
        let Some(li) = next.level else {
            self.diagnostics = diag;
            return self.commit(next);
        };
        let level = &pack.levels[li];
        let index = &self.index[li];

        // ---- which half of the map is friendly
        let (mut at_a, mut at_b) = (0u32, 0u32);
        for (p, aff) in &self.airfields {
            let Some((_, team)) = index.runways.iter().find(|(r, _)| r.dist(*p) < RUNWAY_MATCH_M) else { continue };
            match (aff, team) {
                (Aff::Ally, 1) | (Aff::Enemy, 2) => at_a += 4,
                (Aff::Ally, 2) | (Aff::Enemy, 1) => at_b += 4,
                _ => {}
            }
        }
        // Ground icons: friendly ones stand among their own sites. This only
        // needs "nearer to one half's sites than the other's", so it survives
        // positions being off by hundreds of metres.
        for (position, aff, hits) in self.cells.values() {
            if *hits < STATIC_HITS {
                continue;
            }
            let (da, db) = (nearest(&index.half_a, *position), nearest(&index.half_b, *position));
            if (da - db).abs() < 1500.0 {
                continue;
            }
            let near_a = da < db;
            match aff {
                Aff::Ally | Aff::Player => {
                    if near_a {
                        at_a += 1
                    } else {
                        at_b += 1
                    }
                }
                Aff::Enemy => {
                    if near_a {
                        at_b += 1
                    } else {
                        at_a += 1
                    }
                }
                Aff::Unknown => {}
            }
        }
        if let Some(p) = self.first_player {
            let (da, db) = (nearest(&index.half_a, p), nearest(&index.half_b, p));
            if (da - db).abs() > 3000.0 {
                if da < db {
                    at_a += 3
                } else {
                    at_b += 3
                }
            }
        }
        diag.half_votes = (at_a, at_b);
        let (winner, loser) = (at_a.max(at_b), at_a.min(at_b));
        let ally_at_a = if winner >= 3 && winner >= loser * 2 + 1 { Some(at_a > at_b) } else { None };
        let half_strong = winner >= 10 && winner >= loser * 4;

        // ---- ground-unit fingerprint
        let variants = level.variants.len();
        let (exact, matched_exact) = self.fingerprint(index, variants, eras.len(), MATCH_M, true);
        let (coarse, matched_coarse) = self.fingerprint(index, variants, eras.len(), COARSE_MATCH_M, false);
        diag.matched_exact = matched_exact;
        diag.matched_coarse = matched_coarse;
        let fixed = diag.fixed_friendly + diag.fixed_enemy + diag.fixed_other;
        // Positions that do not line up exactly (but do roughly) mean the
        // mission data is slightly off: fall back to site-level matching.
        let use_coarse = fixed >= 20 && (matched_exact as usize) * 3 < fixed && (matched_coarse as usize) * 2 >= fixed;
        diag.used_coarse = use_coarse;
        let score = if use_coarse { coarse } else { exact };

        // Kill-feed names: a unit that only exists in some brackets votes for them.
        let mut hud_votes = vec![0u32; eras.len()];
        if let Some(variant) = level.variants.first() {
            let mut masks: HashMap<&str, u32> = HashMap::new();
            for site in &variant.sites {
                for unit in &site.units {
                    *masks.entry(unit.u.as_str()).or_insert(0) |= unit.e;
                }
            }
            for id in self.hud_units.keys() {
                if let Some(mask) = masks.get(id.as_str()) {
                    for (ei, vote) in hud_votes.iter_mut().enumerate() {
                        if mask >> ei & 1 == 1 {
                            *vote += 1;
                        }
                    }
                }
            }
        }

        // ---- candidates: (total, evidence from the map alone, variant, team, era)
        let manual_variant = manual.variant.as_ref().and_then(|id| level.variants.iter().position(|v| &v.id == id));
        let manual_era = manual.era.as_ref().and_then(|id| pack.era_index(id));
        let era_allowed = |ei: usize| vehicle_rank.map_or(true, |rank| eras[ei].rank[1] >= rank);
        let mut ranked: Vec<(u32, u32, usize, u8, usize)> = Vec::new();
        for (vi, variant) in level.variants.iter().enumerate() {
            if manual_variant.is_some_and(|m| m != vi) {
                continue;
            }
            for my in 1..=2u8 {
                if manual.team.is_some_and(|m| m != my) {
                    continue;
                }
                if let Some(at_a) = ally_at_a {
                    // In the swapped mission team 1 sits where team 2 normally does.
                    let my_at_a = (my == 1) != variant.mirror;
                    if my_at_a != at_a {
                        continue;
                    }
                }
                for ei in 0..eras.len() {
                    if manual_era.is_some_and(|m| m != ei) || (manual_era.is_none() && !era_allowed(ei)) {
                        continue;
                    }
                    let seen = score[vi][my as usize - 1][ei] + 5 * hud_votes[ei];
                    let bonus = if diag.nation_team == Some(my) { NATION_BONUS } else { 0 };
                    ranked.push((seen + bonus, seen, vi, my, ei));
                }
            }
        }
        if ranked.is_empty() {
            self.diagnostics = diag;
            return self.commit(next);
        }
        ranked.sort_by(|a, b| b.0.cmp(&a.0));
        let best = ranked[0];
        let side = |c: &(u32, u32, usize, u8, usize)| (c.2, c.3);

        // side (layout + team)
        let rival_seen = ranked.iter().filter(|c| side(c) != side(&best)).map(|c| c.1).max().unwrap_or(0);
        let rival_exists = ranked.iter().any(|c| side(c) != side(&best));
        let side_fixed = manual_variant.is_some() && manual.team.is_some();
        let margin = best.1.saturating_sub(rival_seen);
        let nation_agrees = diag.nation_team == Some(best.3);
        next.side_conf = if side_fixed {
            Confidence::Manual
        } else if !use_coarse && best.1 >= 12 && margin >= (best.1 / 6).max(4) {
            Confidence::Confirmed
        } else if !rival_exists && ally_at_a.is_some() {
            Confidence::Likely
        } else if ally_at_a.is_some() && (margin >= 3 || (nation_agrees && half_strong)) {
            Confidence::Likely
        } else {
            Confidence::Guess
        };
        // Show a best guess as soon as there is anything to go on, rather
        // than leaving the map on whatever was selected before.
        if next.side_conf != Confidence::Guess || ally_at_a.is_some() || diag.nation_team.is_some() || best.1 > 0 {
            next.variant = Some(best.2);
            next.team = Some(best.3);
        }
        let layout = if level.variants[best.2].mirror { "swapped" } else { "standard" };
        if side_fixed {
            next.why.push("Side chosen by you.".into());
        } else if next.variant.is_none() {
            next.why.push("Side: nothing to go on yet.".into());
        } else {
            let mut reasons = Vec::new();
            if let Some(at_a) = ally_at_a {
                reasons.push(format!("friendly forces hold the {} half ({} to {} votes)", if at_a { "team-1" } else { "team-2" }, winner, loser));
            }
            if nation_agrees {
                reasons.push(format!("your vehicle's nation ({}) flies for team {}", diag.nation, best.3));
            }
            if best.1 > 0 {
                reasons.push(format!("{} ground units fit{}", best.1, if use_coarse { " (roughly)" } else { "" }));
            }
            next.why.push(format!("Side: team {}, {layout} layout - {}.", best.3, reasons.join("; ")));
        }

        // BR bracket, judged inside the winning side
        let same_side: Vec<&(u32, u32, usize, u8, usize)> = ranked.iter().filter(|c| side(c) == side(&best)).collect();
        let top = same_side.iter().map(|c| c.1).max().unwrap_or(0);
        let mut options: Vec<usize> = same_side.iter().filter(|c| top.saturating_sub(c.1) <= (top / 20).max(1)).map(|c| c.4).collect();
        options.sort();
        options.dedup();
        let vehicle_era = vehicle_rank.and_then(|rank| eras.iter().position(|e| rank >= e.rank[0] && rank <= e.rank[1]));
        if let Some(ei) = manual_era {
            next.era = Some(ei);
            next.era_conf = Confidence::Manual;
            next.why.push("BR bracket chosen by you.".into());
        } else if options.len() == 1 && top >= 8 && !use_coarse {
            next.era = Some(options[0]);
            next.era_conf = if top >= 16 { Confidence::Confirmed } else { Confidence::Likely };
            next.why.push(format!("BR bracket: {top} unit positions fit {} only.", eras[options[0]].label));
        } else {
            // Several brackets fit: prefer the one the player's vehicle sits in,
            // then the lowest one that still fits.
            let pick = vehicle_era.filter(|e| options.contains(e)).or_else(|| options.first().copied());
            next.era = pick;
            next.era_conf = Confidence::Guess;
            next.era_options = options;
            next.why.push(match vehicle_era {
                Some(e) if pick == Some(e) => format!("BR bracket: guessed from your vehicle ({}).", eras[e].label),
                _ => "BR bracket: not enough units seen yet.".into(),
            });
        }
        self.diagnostics = diag;
        self.commit(next)
    }

    fn commit(&mut self, next: Identity) -> bool {
        let changed = (next.level, next.variant, next.team, next.era)
            != (self.identity.level, self.identity.variant, self.identity.team, self.identity.era);
        self.identity = next;
        changed
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}
