//! Works out which battle the player is in: map, which side layout, which
//! team, and which BR bracket - using only what the game shows on its map.
//!
//! * Map: runway positions.
//! * Side: which airfields are painted in the friendly colour.
//! * Layout and BR bracket: every bracket places different air-defence
//!   systems, and the two factions arrange theirs differently, so the pattern
//!   of ground units on the map is a fingerprint. Friendly ground units are
//!   always visible, so this settles within seconds of spawning.
//! * Tie-breakers: unit names in the kill feed and the BR of the player's
//!   own vehicle (a battle cannot be in a bracket below it).

use crate::geo::{Grid, MapFrame, V2};
use crate::pack::Pack;
use crate::telemetry::Aff;
use serde::Serialize;
use std::collections::HashMap;

/// A ground icon must be seen this many times in one spot to count as fixed.
const STATIC_HITS: u16 = 4;
const CELL_M: f64 = 8.0;
const MATCH_M: f64 = 30.0;
const RUNWAY_MATCH_M: f64 = 700.0;

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

/// Pre-indexed placements of one level, built once per pack.
struct LevelIndex {
    /// (variant, team, era mask) for each indexed point.
    meta: Vec<(u8, u8, u32)>,
    grid: Grid,
    /// Runway midpoints with the team that owns them in the first variant.
    runways: Vec<(V2, u8)>,
}

pub struct Identifier {
    index: Vec<LevelIndex>,
    airfields: Vec<(V2, Aff)>,
    cells: HashMap<(i32, i32), (V2, Aff, u16)>,
    hud_units: HashMap<String, u32>,
    last_eval_statics: usize,
    dirty: bool,
    pub identity: Identity,
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
                let runways = level.variants[0]
                    .sites
                    .iter()
                    .flat_map(|site| {
                        site.rw.iter().map(move |r| (V2((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0), site.team))
                    })
                    .collect();
                LevelIndex { meta, grid, runways }
            })
            .collect();
        Identifier {
            index,
            airfields: Vec::new(),
            cells: HashMap::new(),
            hud_units: HashMap::new(),
            last_eval_statics: 0,
            dirty: true,
            identity: Identity::default(),
        }
    }

    pub fn reset(&mut self) {
        self.airfields.clear();
        self.cells.clear();
        self.hud_units.clear();
        self.last_eval_statics = 0;
        self.dirty = true;
        self.identity = Identity::default();
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

    pub fn see_kill(&mut self, unit_ids: &[String]) {
        for id in unit_ids {
            *self.hud_units.entry(id.clone()).or_insert(0) += 1;
        }
        self.dirty = true;
    }

    /// Re-evaluates when new evidence arrived. Returns true if the identity
    /// (not just its confidence) changed.
    pub fn evaluate(&mut self, pack: &Pack, frame: Option<MapFrame>, vehicle_rank: Option<i32>, manual: &Manual) -> bool {
        if !self.dirty {
            return false;
        }
        self.dirty = false;
        let mut next = Identity::default();
        let eras = &pack.core.eras;

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
            } else if level_scores.len() == 1 && frame.is_some() {
                next.level = Some(li);
                next.level_conf = Confidence::Guess;
                next.why.push(format!("Map: only {} has this size.", pack.levels[li].name));
            }
        }
        let Some(li) = next.level else {
            return self.commit(next);
        };
        let level = &pack.levels[li];
        let index = &self.index[li];

        // ---- which half of the map is friendly
        let (mut at_a, mut at_b) = (0, 0);
        for (p, aff) in &self.airfields {
            let Some((_, team)) = index.runways.iter().find(|(r, _)| r.dist(*p) < RUNWAY_MATCH_M) else { continue };
            match (aff, team) {
                (Aff::Ally, 1) | (Aff::Enemy, 2) => at_a += 1,
                (Aff::Ally, 2) | (Aff::Enemy, 1) => at_b += 1,
                _ => {}
            }
        }
        let ally_at_a = if at_a > at_b { Some(true) } else if at_b > at_a { Some(false) } else { None };

        // ---- ground-unit fingerprint: score[variant][my team - 1][era]
        let variants = level.variants.len();
        let mut score = vec![[vec![0u32; eras.len()], vec![0u32; eras.len()]]; variants];
        let mut statics = 0usize;
        for (position, aff, hits) in self.cells.values() {
            if *hits < STATIC_HITS {
                continue;
            }
            statics += 1;
            // era masks of matching placements per (variant, unit team)
            let mut found = vec![[0u32; 2]; variants];
            index.grid.each_near(*position, MATCH_M, |i, _| {
                let (vi, team, mask) = index.meta[i as usize];
                if (1..=2).contains(&team) {
                    found[vi as usize][team as usize - 1] |= mask;
                }
            });
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
        self.last_eval_statics = statics;

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
            let mut missing = vec![[vec![0u32; eras.len()], vec![0u32; eras.len()]]; variants];
            for (i, &(vi, team, mask)) in index.meta.iter().enumerate() {
                if !(1..=2).contains(&team) {
                    continue;
                }
                let position = index.grid.point(i as u32);
                if seen.nearest(position, MATCH_M).is_some() {
                    continue;
                }
                for (ei, slot) in missing[vi as usize][team as usize - 1].iter_mut().enumerate() {
                    if mask >> ei & 1 == 1 {
                        *slot += 1;
                    }
                }
            }
            for vi in 0..variants {
                for my in 0..2usize {
                    for ei in 0..eras.len() {
                        score[vi][my][ei] = score[vi][my][ei].saturating_sub(missing[vi][my][ei]);
                    }
                }
            }
        }

        // Kill-feed names: a unit that only exists in some brackets votes for them.
        let mut hud_votes = vec![0u32; eras.len()];
        for variant in &level.variants {
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
            break; // unit types per bracket are the same in every layout
        }

        // ---- candidates
        let manual_variant = manual.variant.as_ref().and_then(|id| level.variants.iter().position(|v| &v.id == id));
        let manual_era = manual.era.as_ref().and_then(|id| pack.era_index(id));
        let era_allowed = |ei: usize| vehicle_rank.map_or(true, |rank| eras[ei].rank[1] >= rank);
        let mut ranked: Vec<(u32, usize, u8, usize)> = Vec::new();
        for (vi, variant) in level.variants.iter().enumerate() {
            if manual_variant.is_some_and(|m| m != vi) {
                continue;
            }
            for my in 1..=2u8 {
                if manual.team.is_some_and(|m| m != my) {
                    continue;
                }
                if let Some(at_a) = ally_at_a {
                    // In the mirrored mission team 1 sits where team 2 normally does.
                    let my_at_a = (my == 1) != variant.mirror;
                    if my_at_a != at_a {
                        continue;
                    }
                }
                for ei in 0..eras.len() {
                    if manual_era.is_some_and(|m| m != ei) || (manual_era.is_none() && !era_allowed(ei)) {
                        continue;
                    }
                    ranked.push((score[vi][my as usize - 1][ei] + 5 * hud_votes[ei], vi, my, ei));
                }
            }
        }
        if ranked.is_empty() {
            return self.commit(next);
        }
        ranked.sort_by(|a, b| b.0.cmp(&a.0));
        let best = ranked[0];

        // side (variant + team)
        let side_rival = ranked.iter().find(|c| (c.1, c.2) != (best.1, best.2)).map_or(0, |c| c.0);
        let side_fixed = manual_variant.is_some() && manual.team.is_some();
        let side_margin = best.0.saturating_sub(side_rival);
        next.side_conf = if side_fixed {
            Confidence::Manual
        } else if best.0 >= 12 && side_margin >= (best.0 / 6).max(4) {
            Confidence::Confirmed
        } else if best.0 >= 5 && side_margin >= 3 {
            Confidence::Likely
        } else if ranked.iter().all(|c| (c.1, c.2) == (best.1, best.2)) && ally_at_a.is_some() {
            Confidence::Likely
        } else {
            Confidence::Guess
        };
        if next.side_conf != Confidence::Guess || ally_at_a.is_some() {
            next.variant = Some(best.1);
            next.team = Some(best.2);
        }
        if side_fixed {
            next.why.push("Side chosen by you.".into());
        } else if next.side_conf == Confidence::Guess {
            next.why.push("Side: waiting for more ground units to appear on the map.".into());
        } else {
            next.why.push(format!(
                "Side: {} ground units match the {} layout as team {}.",
                best.0,
                if level.variants[best.1].mirror { "swapped" } else { "standard" },
                best.2
            ));
        }

        // BR bracket, judged inside the winning side
        let same_side: Vec<&(u32, usize, u8, usize)> = ranked.iter().filter(|c| (c.1, c.2) == (best.1, best.2)).collect();
        let top = same_side[0].0;
        let mut options: Vec<usize> = same_side.iter().filter(|c| top.saturating_sub(c.0) <= (top / 20).max(1)).map(|c| c.3).collect();
        options.sort();
        let vehicle_era = vehicle_rank.and_then(|rank| eras.iter().position(|e| rank >= e.rank[0] && rank <= e.rank[1]));
        if let Some(ei) = manual_era {
            next.era = Some(ei);
            next.era_conf = Confidence::Manual;
            next.why.push("BR bracket chosen by you.".into());
        } else if options.len() == 1 && top >= 8 {
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
        self.commit(next)
    }

    fn commit(&mut self, next: Identity) -> bool {
        let changed = (next.level, next.variant, next.team, next.era)
            != (self.identity.level, self.identity.variant, self.identity.team, self.identity.era);
        self.identity = next;
        changed
    }

    pub fn statics_seen(&self) -> usize {
        self.last_eval_statics
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}
