//! The engine: takes what the game reports, keeps the picture of the battle,
//! and hands the interface a snapshot to draw.

use crate::convoy::{Contact, ConvoyState, Convoys, RouteKind};
use crate::geo::{Grid, MapFrame, V2};
use crate::ident::{Confidence, Identifier, Manual};
use crate::pack::{Pack, Site};
use crate::roads::RoadGraph;
use crate::telemetry::{affiliation, Aff, Flight, Frame, Kind, MapInfo, Vehicle};
use crate::world::{parse_kill, KillOutcome, Proof, Status, World};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The player uses inverted friend/foe colours in the game.
    pub colors_swapped: bool,
    /// Radius used to judge "many enemies in one place".
    pub strike_radius_m: f64,
    /// Fewest enemy units inside the radius worth pointing out.
    pub strike_min_units: u32,
    /// How far ahead to announce a column reaching a site.
    pub alert_lead_s: f64,
    pub manual_level: Option<String>,
    pub manual_variant: Option<String>,
    pub manual_team: Option<u8>,
    pub manual_era: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            colors_swapped: false,
            strike_radius_m: 1000.0,
            strike_min_units: 6,
            alert_lead_s: 120.0,
            manual_level: None,
            manual_variant: None,
            manual_team: None,
            manual_era: None,
        }
    }
}

impl Settings {
    fn manual(&self) -> Manual {
        Manual {
            level: self.manual_level.clone(),
            variant: self.manual_variant.clone(),
            team: self.manual_team,
            era: self.manual_era.clone(),
        }
    }
    fn sanitize(&mut self) {
        self.strike_radius_m = self.strike_radius_m.clamp(200.0, 10_000.0);
        self.strike_min_units = self.strike_min_units.clamp(2, 100);
        self.alert_lead_s = self.alert_lead_s.clamp(10.0, 600.0);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// Seconds since the app started.
    pub at: f64,
    /// Battle clock, when known.
    pub battle_t: Option<f64>,
    pub kind: &'static str,
    pub text: String,
    pub site: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerView {
    pub p: V2,
    pub dir: V2,
    pub alt_m: Option<f64>,
    pub tas_kmh: Option<f64>,
    pub vehicle: Option<String>,
    pub vehicle_name: Option<String>,
    pub br: Option<f32>,
    pub grid: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContactView {
    pub p: V2,
    pub dir: V2,
    /// "ally", "enemy" or "unknown"
    pub aff: &'static str,
    /// "air", "ground" or "zone"
    pub kind: &'static str,
    pub icon: String,
    pub blink: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConvoyView {
    pub id: u32,
    pub team: u8,
    pub enemy: bool,
    pub n: u32,
    pub p: V2,
    pub dir: V2,
    pub speed: f64,
    pub state: ConvoyState,
    pub kind: Option<RouteKind>,
    pub from: Option<usize>,
    pub to: Option<usize>,
    /// Stretch of road the column is probably on (flat x, z list).
    pub band: Vec<f64>,
    /// Road still ahead (flat x, z list).
    pub ahead: Vec<f64>,
    pub eta_s: Option<f64>,
    pub unseen_s: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertView {
    pub id: String,
    /// "stack": enemy column at an enemy site. "attack": enemy column at one of yours.
    pub kind: &'static str,
    /// "now" or "soon"
    pub phase: &'static str,
    pub site: usize,
    pub convoy: u32,
    pub p: V2,
    pub r: f64,
    pub count: u32,
    pub eta_s: f64,
    pub uncertain: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpportunityView {
    pub p: V2,
    pub r: f64,
    pub count: u32,
    pub friendly: u32,
    pub site: Option<usize>,
    pub convoys: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CueView {
    pub site: usize,
    pub p: V2,
    pub dist_m: f64,
    pub bearing: f64,
    /// Bearing relative to the nose: -180..180, negative = left.
    pub relative: Option<f64>,
    pub reach_m: f64,
    pub inside: bool,
    pub alive: u32,
    pub total: u32,
    pub grid: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BattleView {
    pub level: Option<String>,
    pub variant: Option<String>,
    pub team: Option<u8>,
    pub era: Option<String>,
    pub level_conf: Confidence,
    pub side_conf: Confidence,
    pub era_conf: Confidence,
    pub era_options: Vec<String>,
    pub why: Vec<String>,
    pub clock_s: Option<f64>,
    pub unlocated_kills: usize,
    pub mission_status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduleView {
    pub site: usize,
    /// Seconds until the next dispatch window opens and closes.
    pub next_in: (f64, f64),
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub seq: u64,
    /// "offline" (game not running), "menu" (running, no battle) or "battle".
    pub link: &'static str,
    /// In a battle on a map the data pack does not know.
    pub unknown_map: bool,
    pub bounds: Option<[f64; 4]>,
    pub battle: BattleView,
    pub player: Option<PlayerView>,
    pub contacts: Vec<ContactView>,
    /// Fixed units with a known state: [site, slot, status, proof, on map now, seconds until restore].
    pub units: Vec<(u16, u16, Status, Proof, bool, Option<i32>)>,
    pub convoys: Vec<ConvoyView>,
    pub alerts: Vec<AlertView>,
    pub opportunities: Vec<OpportunityView>,
    pub cues: Vec<CueView>,
    pub schedules: Vec<ScheduleView>,
    pub log: Vec<LogEntry>,
    pub settings: Settings,
}

struct Other {
    p: V2,
    dir: V2,
    aff: Aff,
    kind: &'static str,
    icon: String,
    blink: bool,
}

pub struct Engine {
    pub pack: Arc<Pack>,
    settings: Settings,
    ident: Identifier,
    world: Option<World>,
    convoys: Convoys,
    convoy_team: HashMap<String, u8>,
    now: f64,
    reachable: bool,
    battle_seen_at: f64,
    no_battle_since: Option<f64>,
    map_info: Option<MapInfo>,
    generation: Option<i64>,
    vehicle: Option<Vehicle>,
    flight: Flight,
    player: Option<(V2, V2)>,
    others: Vec<Other>,
    clock_offset: Option<f64>,
    last_hud_time: f64,
    last_hud_id: i64,
    mission_status: String,
    log: VecDeque<LogEntry>,
    seq: u64,
}

fn site_label(site: &Site) -> String {
    let kind = match site.cat.as_str() {
        "airfield" => "Airfield",
        "strongpoint" => "Strongpoint",
        "assembly_area" => "Vehicle depot",
        "fuel_factory" => "Fuel depot",
        "ammo_factory" => "Ammo depot",
        "long_sam" => "Long-range SAM",
        "short_sam" => "SAM site",
        "sam_cover" => "SAM escort",
        "spaa_cover" => "SPAA post",
        "ew_radar" => "Early-warning radar",
        "ship" => "Ship",
        "mlrs" => "MLRS",
        "tbm" => "Missile launcher",
        _ => "Site",
    };
    let number: String = site.id.rsplit('_').next().filter(|t| t.chars().all(|c| c.is_ascii_digit())).unwrap_or("").into();
    if site.id.contains("airfield_main") {
        "Main airfield".into()
    } else if number.is_empty() {
        kind.into()
    } else {
        format!("{kind} {number}")
    }
}

impl Engine {
    pub fn new(pack: Arc<Pack>) -> Engine {
        // Which team each convoy vehicle type belongs to, from the templates.
        fn walk(value: &serde_json::Value, team: u8, out: &mut HashMap<String, u8>) {
            match value {
                serde_json::Value::Object(map) => {
                    if let (Some(name), true) = (map.get("name").and_then(|n| n.as_str()), team != 0) {
                        out.insert(name.to_string(), team);
                    }
                    for (key, child) in map {
                        let next = match key.as_str() {
                            "redfor" => 1,
                            "blufor" => 2,
                            _ => team,
                        };
                        walk(child, next, out);
                    }
                }
                serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, team, out)),
                _ => {}
            }
        }
        let mut convoy_team = HashMap::new();
        walk(&pack.core.convoys, 0, &mut convoy_team);
        Engine {
            ident: Identifier::new(&pack),
            pack,
            settings: Settings::default(),
            world: None,
            convoys: Convoys::new(),
            convoy_team,
            now: 0.0,
            reachable: false,
            battle_seen_at: f64::NEG_INFINITY,
            no_battle_since: None,
            map_info: None,
            generation: None,
            vehicle: None,
            flight: Flight::default(),
            player: None,
            others: Vec::new(),
            clock_offset: None,
            last_hud_time: f64::NEG_INFINITY,
            last_hud_id: 0,
            mission_status: String::new(),
            log: VecDeque::new(),
            seq: 0,
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn set_settings(&mut self, mut settings: Settings) {
        settings.sanitize();
        let radius_changed = (settings.strike_radius_m - self.settings.strike_radius_m).abs() > 1.0;
        self.settings = settings;
        self.ident.mark_dirty();
        if radius_changed {
            self.world = None; // routes remember which sites they pass; rebuild
        }
        self.reidentify();
        self.seq += 1;
    }

    /// Forgets everything about the current battle.
    pub fn reset_battle(&mut self) {
        self.ident.reset();
        self.world = None;
        self.convoys = Convoys::new();
        self.clock_offset = None;
        self.last_hud_time = f64::NEG_INFINITY;
        self.player = None;
        self.others.clear();
        self.seq += 1;
    }

    pub fn mark(&mut self, site: u16, slot: Option<u16>, destroyed: Option<bool>) {
        let now = self.now;
        if let Some(world) = &mut self.world {
            world.mark(site, slot, destroyed, now);
            self.seq += 1;
        }
    }

    fn battle_t(&self) -> Option<f64> {
        self.clock_offset.map(|offset| (self.now - offset).max(0.0))
    }

    fn push_log(&mut self, kind: &'static str, text: String, site: Option<u16>) {
        self.log.push_back(LogEntry { at: self.now, battle_t: self.battle_t(), kind, text, site });
        while self.log.len() > 200 {
            self.log.pop_front();
        }
    }

    fn frame(&self) -> Option<MapFrame> {
        if let Some(info) = &self.map_info {
            return Some(MapFrame { min: V2(info.min[0], info.min[1]), max: V2(info.max[0], info.max[1]) });
        }
        self.ident.identity.level.map(|li| MapFrame::from_bounds(self.pack.levels[li].bounds))
    }

    fn vehicle_rank(&self) -> Option<i32> {
        let vehicle = self.vehicle.as_ref()?;
        self.pack.core.vehicles.get(&vehicle.id).map(|entry| entry.0)
    }

    fn reidentify(&mut self) {
        let frame = self.map_info.as_ref().map(|info| MapFrame { min: V2(info.min[0], info.min[1]), max: V2(info.max[0], info.max[1]) });
        let rank = self.vehicle_rank();
        let changed = self.ident.evaluate(&self.pack, frame, rank, &self.settings.manual());
        let identity = &self.ident.identity;
        let wanted = match (identity.level, identity.variant, identity.team, identity.era) {
            (Some(l), Some(v), Some(t), Some(e)) => Some((l, v, e, t)),
            _ => None,
        };
        let current = self.world.as_ref().map(|w| (w.level, w.variant, w.era, w.my_team));
        if wanted == current && !changed {
            return;
        }
        if wanted == current {
            return;
        }
        match wanted {
            Some((level, variant, era, team)) => {
                let same_layout = current.is_some_and(|c| (c.0, c.1) == (level, variant));
                self.world = Some(World::build(&self.pack, level, variant, era, team));
                if !same_layout {
                    let lvl = &self.pack.levels[level];
                    let mut graph = RoadGraph::build(&lvl.road_file);
                    let pass_radius = self.settings.strike_radius_m * 0.9;
                    self.convoys.prepare(&lvl.variants[variant], &mut graph, &self.pack.core.rules.resource_factory, pass_radius);
                }
                let era_label = self.pack.core.eras[era].label.clone();
                let name = self.pack.levels[level].name.clone();
                self.push_log("info", format!("Battle recognised: {name}, {era_label}, team {team}."), None);
            }
            None => self.world = None,
        }
    }

    /// Feeds one poll of the game. `now` is seconds on any steady clock.
    pub fn ingest(&mut self, frame: &Frame, now: f64) {
        self.now = now;
        self.reachable = frame.reachable;
        self.seq += 1;
        if !frame.reachable {
            self.convoys.update(now, &[], false, self.battle_t());
            return;
        }
        if let Some(status) = &frame.mission_status {
            self.mission_status = status.clone();
        }
        if let Some(vehicle) = &frame.vehicle {
            if vehicle.is_some() && *vehicle != self.vehicle {
                self.vehicle = vehicle.clone();
                self.ident.mark_dirty();
            }
        }
        if let Some(flight) = &frame.flight {
            self.flight = flight.clone();
        }
        if let Some(info) = &frame.map_info {
            match info {
                Some(info) => {
                    if self.generation.is_some_and(|g| g != info.generation) {
                        self.reset_battle();
                        self.push_log("info", "New battle detected.".into(), None);
                    }
                    self.generation = Some(info.generation);
                    if self.map_info.as_ref() != Some(info) {
                        self.map_info = Some(info.clone());
                        self.ident.mark_dirty();
                    }
                    self.no_battle_since = None;
                }
                None => {
                    let since = *self.no_battle_since.get_or_insert(now);
                    if now - since > 3.0 && self.map_info.is_some() {
                        self.map_info = None;
                        self.generation = None;
                        self.reset_battle();
                        self.push_log("info", "Battle ended.".into(), None);
                    }
                }
            }
        }

        // The kill feed first: its lines explain icons that vanish this frame.
        for line in &frame.hud {
            if let Some(time) = line.time {
                if time < self.last_hud_time - 60.0 {
                    self.reset_battle();
                    self.push_log("info", "New battle detected.".into(), None);
                }
                self.last_hud_time = time;
                let offset = now - time;
                self.clock_offset = Some(self.clock_offset.map_or(offset, |old| old.min(offset)));
            }
            self.last_hud_id = self.last_hud_id.max(line.id);
            let Some(ids) = parse_kill(&line.text, &self.pack) else { continue };
            self.ident.see_kill(&ids);
            let Some(world) = &mut self.world else { continue };
            let my_team = world.my_team;
            match world.kill_feed(&ids, now, &self.pack.core.units) {
                KillOutcome::Located(site, name) => {
                    let s = &self.pack.levels[world.level].variants[world.variant].sites[site as usize];
                    let side = if s.team == my_team { "friendly" } else { "enemy" };
                    let text = format!("{name} destroyed at {side} {}.", site_label(s).to_lowercase());
                    self.push_log("kill", text, Some(site));
                }
                KillOutcome::Unlocated(name) => {
                    self.push_log("kill", format!("{name} destroyed - which one is not known yet."), None);
                }
                KillOutcome::NotFixed => {
                    if let Some(team) = ids.iter().find_map(|id| self.convoy_team.get(id)).copied() {
                        if self.convoys.kill(team, now).is_some() && team != my_team {
                            let name = self.pack.core.units.get(&ids[0]).map_or(ids[0].as_str(), |u| u.short()).to_string();
                            self.push_log("kill", format!("{name} destroyed in an enemy column."), None);
                        }
                    }
                }
            }
        }

        let mut contacts: Vec<Contact> = Vec::new();
        let fresh = frame.objects.is_some();
        if let Some(objects) = &frame.objects {
            if !objects.is_empty() {
                self.battle_seen_at = now;
            }
            if let Some(map) = self.frame() {
                let swapped = self.settings.colors_swapped;
                let mut airfields = Vec::new();
                self.player = None;
                self.others.clear();
                let mut ground: Vec<(V2, Aff, &str, bool)> = Vec::new();
                for obj in objects {
                    let aff = affiliation(obj, swapped);
                    let p = map.to_world(obj.x, obj.y);
                    let dir = V2(obj.dx, -obj.dy).unit();
                    match obj.kind {
                        Kind::Airfield => {
                            let end = map.to_world(obj.ex, obj.ey);
                            airfields.push((p.lerp(end, 0.5), aff));
                        }
                        Kind::Ground => {
                            self.ident.see_ground(p, aff);
                            ground.push((p, aff, obj.icon.as_str(), obj.blink));
                        }
                        Kind::Aircraft if aff == Aff::Player => self.player = Some((p, dir)),
                        Kind::Aircraft => self.others.push(Other { p, dir, aff, kind: "air", icon: obj.icon.clone(), blink: obj.blink }),
                        Kind::Zone | Kind::Other => {
                            if aff == Aff::Player {
                                self.player = Some((p, dir));
                            } else {
                                self.others.push(Other { p, dir, aff, kind: "zone", icon: obj.icon.clone(), blink: obj.blink })
                            }
                        }
                    }
                }
                self.ident.see_airfields(airfields);
                self.reidentify();
                let my_team = self.world.as_ref().map(|w| w.my_team);
                for (p, aff, icon, blink) in ground {
                    let team = match (aff, my_team) {
                        (Aff::Ally | Aff::Player, Some(t)) => Some(t),
                        (Aff::Enemy, Some(t)) => Some(3 - t),
                        _ => None,
                    };
                    if aff == Aff::Player {
                        self.player = Some((p, V2(0.0, 0.0)));
                    }
                    let matched = self.world.as_ref().and_then(|w| w.match_icon(p, team));
                    match (matched, &mut self.world) {
                        (Some(index), Some(world)) => {
                            if world.see(index, now) {
                                let site = world.units[index as usize].site;
                                let s = &self.pack.levels[world.level].variants[world.variant].sites[site as usize];
                                let text = format!("A unit is back at {}.", site_label(s).to_lowercase());
                                self.push_log("info", text, Some(site));
                            }
                        }
                        _ => {
                            contacts.push(Contact { pos: p, team: team.unwrap_or(0) });
                            self.others.push(Other { p, dir: V2(0.0, 0.0), aff, kind: "ground", icon: icon.to_string(), blink });
                        }
                    }
                }
            }
            if let Some(world) = &mut self.world {
                let my_team = world.my_team;
                let changed = world.settle(now);
                let mut lines = Vec::new();
                for index in changed {
                    let unit = &world.units[index as usize];
                    if unit.proof != Proof::Likely || unit.team == my_team {
                        continue;
                    }
                    let s = &self.pack.levels[world.level].variants[world.variant].sites[unit.site as usize];
                    let name = self.pack.core.units.get(&unit.id).map_or(unit.id.as_str(), |u| u.short());
                    lines.push((format!("{name} at {} left the map - probably destroyed.", site_label(s).to_lowercase()), unit.site));
                }
                for (text, site) in lines {
                    self.push_log("lost", text, Some(site));
                }
            }
        }
        self.convoys.update(now, &contacts, fresh, self.battle_t());
    }

    fn grid_ref(&self, p: V2) -> Option<String> {
        let info = self.map_info.as_ref()?;
        let (zero, steps) = (info.grid_zero?, info.grid_steps?);
        if steps[0] <= 0.0 || steps[1] <= 0.0 {
            return None;
        }
        let column = ((p.0 - zero[0]) / steps[0]).floor() as i64;
        let row = ((zero[1] - p.1) / steps[1]).floor() as i64;
        if !(0..26).contains(&row) || !(0..99).contains(&column) {
            return None;
        }
        Some(format!("{}{}", (b'A' + row as u8) as char, column + 1))
    }

    pub fn snapshot(&self) -> Snapshot {
        let now = self.now;
        let identity = &self.ident.identity;
        let eras = &self.pack.core.eras;
        let in_battle = self.reachable && self.map_info.is_some() && now - self.battle_seen_at < 15.0;
        let link = if !self.reachable {
            "offline"
        } else if in_battle {
            "battle"
        } else {
            "menu"
        };
        let level = identity.level.map(|li| &self.pack.levels[li]);
        let battle = BattleView {
            level: level.map(|l| l.id.clone()),
            variant: identity.variant.and_then(|vi| level.map(|l| l.variants[vi].id.clone())),
            team: identity.team,
            era: identity.era.map(|ei| eras[ei].id.clone()),
            level_conf: identity.level_conf,
            side_conf: identity.side_conf,
            era_conf: identity.era_conf,
            era_options: identity.era_options.iter().map(|&ei| eras[ei].id.clone()).collect(),
            why: identity.why.clone(),
            clock_s: self.battle_t(),
            unlocated_kills: self.world.as_ref().map_or(0, World::unlocated_kills),
            mission_status: self.mission_status.clone(),
        };

        let vehicle_entry = self.vehicle.as_ref().and_then(|v| self.pack.core.vehicles.get(&v.id));
        let player = self.player.map(|(p, dir)| PlayerView {
            p: p.round(),
            dir,
            alt_m: self.flight.altitude_m,
            tas_kmh: self.flight.tas_kmh,
            vehicle: self.vehicle.as_ref().map(|v| v.id.clone()),
            vehicle_name: vehicle_entry.map(|e| e.3.clone()).filter(|n| !n.is_empty()),
            br: vehicle_entry.map(|e| ((1.0 + e.0 as f32 / 3.0) * 10.0).round() / 10.0),
            grid: self.grid_ref(p),
        });

        let aff_name = |aff: Aff| match aff {
            Aff::Ally | Aff::Player => "ally",
            Aff::Enemy => "enemy",
            Aff::Unknown => "unknown",
        };
        let contacts = self
            .others
            .iter()
            .take(1500)
            .map(|o| ContactView { p: o.p.round(), dir: o.dir, aff: aff_name(o.aff), kind: o.kind, icon: o.icon.clone(), blink: o.blink })
            .collect();

        let mut snapshot = Snapshot {
            seq: self.seq,
            link,
            unknown_map: in_battle && identity.level.is_none(),
            bounds: self.map_info.as_ref().map(|i| [i.min[0], i.min[1], i.max[0], i.max[1]]),
            battle,
            player,
            contacts,
            units: Vec::new(),
            convoys: Vec::new(),
            alerts: Vec::new(),
            opportunities: Vec::new(),
            cues: Vec::new(),
            schedules: Vec::new(),
            log: self.log.iter().rev().take(60).cloned().collect(),
            settings: self.settings.clone(),
        };
        let Some(world) = &self.world else { return snapshot };
        let variant = &self.pack.levels[world.level].variants[world.variant];
        let enemy_team = 3 - world.my_team;
        let radius = self.settings.strike_radius_m;

        for unit in &world.units {
            if unit.status == Status::Unknown {
                continue;
            }
            let eta = world.restore_eta(unit, now).map(|s| s.round() as i32);
            snapshot.units.push((unit.site, unit.slot, unit.status, unit.proof, unit.visible(now), eta));
        }

        // ---- convoys
        let mut all = self.convoys.list.clone();
        if let Some(t) = self.battle_t() {
            all.extend(self.convoys.scheduled(t, enemy_team));
        }
        let flat = |points: Vec<V2>| points.into_iter().flat_map(|p| [p.0.round(), p.1.round()]).collect::<Vec<f64>>();
        for convoy in &all {
            let route = convoy.route.map(|ri| &self.convoys.routes[ri]);
            let (band, ahead, eta) = match route {
                Some(route) => {
                    let length = route.path.length();
                    let band = if convoy.unc > 1.0 { flat(route.path.slice(convoy.s_now - convoy.unc, convoy.s_now + convoy.unc)) } else { Vec::new() };
                    (band, flat(route.path.slice(convoy.s_now, length)), Some(self.convoys.eta(convoy, length).max(0.0).round()))
                }
                None => (Vec::new(), Vec::new(), None),
            };
            snapshot.convoys.push(ConvoyView {
                id: convoy.id,
                team: convoy.team,
                enemy: convoy.team == enemy_team,
                n: convoy.n,
                p: convoy.pos.round(),
                dir: convoy.dir,
                speed: (convoy.speed * 10.0).round() / 10.0,
                state: convoy.state,
                kind: route.map(|r| r.kind),
                from: route.map(|r| r.from),
                to: route.map(|r| r.to),
                band,
                ahead,
                eta_s: eta,
                unseen_s: if convoy.state == ConvoyState::Predicted { Some((now - convoy.last_seen).round()) } else { None },
            });
        }

        // ---- where the enemy is bunched up
        let mut enemy_grid = Grid::new(radius.max(100.0));
        let mut friendly_grid = Grid::new(radius.max(100.0));
        let mut site_alive = vec![(0u32, 0u32); variant.sites.len()];
        for unit in &world.units {
            site_alive[unit.site as usize].1 += 1;
            if !unit.counts_as_alive() {
                continue;
            }
            site_alive[unit.site as usize].0 += 1;
            if unit.team == enemy_team {
                enemy_grid.insert(unit.pos);
            } else {
                friendly_grid.insert(unit.pos);
            }
        }
        let count_at = |grid: &Grid, p: V2| {
            let mut n = 0u32;
            grid.each_near(p, radius, |_, _| n += 1);
            n
        };
        let enemy_convoys: Vec<&crate::convoy::Convoy> = all.iter().filter(|c| c.team == enemy_team && c.state != ConvoyState::Scheduled).collect();
        let mut candidates: Vec<(V2, Option<usize>)> = variant
            .sites
            .iter()
            .enumerate()
            .filter(|(_, s)| s.team == enemy_team && s.moves.is_none())
            .map(|(si, s)| (s.p, Some(si)))
            .collect();
        candidates.extend(enemy_convoys.iter().map(|c| (c.pos, None)));
        let mut found: Vec<OpportunityView> = Vec::new();
        for (centre, site) in candidates {
            let fixed = count_at(&enemy_grid, centre);
            let near: Vec<&&crate::convoy::Convoy> = enemy_convoys.iter().filter(|c| c.pos.dist(centre) <= radius).collect();
            let count = fixed + near.iter().map(|c| c.n).sum::<u32>();
            if count < self.settings.strike_min_units {
                continue;
            }
            // A plain site with nothing extra is not news unless it is big.
            if near.is_empty() && site.is_some() && count < self.settings.strike_min_units + 2 {
                continue;
            }
            found.push(OpportunityView {
                p: centre.round(),
                r: radius,
                count,
                friendly: count_at(&friendly_grid, centre),
                site,
                convoys: near.iter().map(|c| c.id).collect(),
            });
        }
        found.sort_by(|a, b| (b.convoys.len().min(1), b.count).cmp(&(a.convoys.len().min(1), a.count)));
        for item in found {
            if snapshot.opportunities.iter().all(|kept: &OpportunityView| kept.p.dist(item.p) > radius * 0.7) {
                snapshot.opportunities.push(item);
            }
            if snapshot.opportunities.len() >= 6 {
                break;
            }
        }

        // ---- a column next to a depot or strongpoint
        for convoy in all.iter().filter(|c| c.team == enemy_team) {
            let Some(ri) = convoy.route else { continue };
            let route = &self.convoys.routes[ri];
            for &(si, s_site, _) in &route.passes {
                let site = &variant.sites[si];
                let inside = convoy.pos.dist(site.p) <= radius && convoy.state != ConvoyState::Scheduled;
                let eta = self.convoys.eta(convoy, s_site);
                let soon = eta > 0.0 && eta <= self.settings.alert_lead_s;
                if !(inside || soon) {
                    continue;
                }
                let stack = site.team == enemy_team;
                let count = convoy.n + if stack { site_alive[si].0 } else { 0 };
                snapshot.alerts.push(AlertView {
                    id: format!("{}-{}", convoy.id, si),
                    kind: if stack { "stack" } else { "attack" },
                    phase: if inside { "now" } else { "soon" },
                    site: si,
                    convoy: convoy.id,
                    p: site.p,
                    r: radius,
                    count,
                    eta_s: if inside { 0.0 } else { eta.round() },
                    uncertain: convoy.state != ConvoyState::Observed && convoy.unc > radius * 0.6,
                });
            }
        }
        snapshot.alerts.sort_by(|a, b| (a.phase != "now", a.eta_s as i64).cmp(&(b.phase != "now", b.eta_s as i64)));
        snapshot.alerts.truncate(8);

        // ---- nearest live air defences
        if let Some((p, dir)) = self.player {
            for (si, site) in variant.sites.iter().enumerate() {
                if site.team != enemy_team || !site.is_air_defence() || site_alive[si].0 == 0 {
                    continue;
                }
                let reach = world.site_units[si]
                    .iter()
                    .map(|&ui| &world.units[ui as usize])
                    .filter(|u| u.counts_as_alive())
                    .filter_map(|u| self.pack.core.units.get(&u.id))
                    .map(|spec| spec.reach())
                    .fold(0.0, f64::max);
                let offset = site.p.sub(p);
                let bearing = offset.bearing();
                let relative = if dir.len() > 0.1 { Some(((bearing - dir.bearing() + 540.0) % 360.0 - 180.0).round()) } else { None };
                snapshot.cues.push(CueView {
                    site: si,
                    p: site.p,
                    dist_m: offset.len().round(),
                    bearing: bearing.round(),
                    relative,
                    reach_m: reach,
                    inside: reach > 0.0 && offset.len() <= reach,
                    alive: site_alive[si].0,
                    total: site_alive[si].1,
                    grid: self.grid_ref(site.p),
                });
            }
            snapshot.cues.sort_by(|a, b| a.dist_m.partial_cmp(&b.dist_m).unwrap());
            snapshot.cues.truncate(10);
        }

        if let Some(t) = self.battle_t() {
            for schedule in self.convoys.schedules.iter().filter(|s| s.team == enemy_team) {
                let (lo, hi) = schedule.next(t);
                snapshot.schedules.push(ScheduleView { site: schedule.site, next_in: ((lo - t).round(), (hi - t).round()), locked: schedule.lock.is_some() });
            }
        }
        snapshot
    }

    /// Ground truth helpers for tests and diagnostics.
    pub fn world(&self) -> Option<&World> {
        self.world.as_ref()
    }
    pub fn identity(&self) -> &crate::ident::Identity {
        &self.ident.identity
    }
    pub fn convoys(&self) -> &Convoys {
        &self.convoys
    }
}
