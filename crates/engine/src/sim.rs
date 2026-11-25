//! A stand-in for the game: plays out a Nuclear Escalation battle from the
//! real mission data and answers the same pages the game's local web
//! interface serves. Used by the tests, by the demo mode and by the
//! development server, so the whole app can be exercised without the game.

use crate::geo::{Path, V2};
use crate::pack::Pack;
use crate::roads::RoadGraph;
use crate::telemetry::{self, Frame};
use serde_json::json;
use std::sync::Arc;

const ALLY: &str = "#174DFF";
const ENEMY: &str = "#fa0C00";
const DETECT_LINGER_S: f64 = 10.0;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
    fn pick(&mut self, n: usize) -> usize {
        ((self.next() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

pub struct SimUnit {
    pub site: u16,
    pub slot: u16,
    pub team: u8,
    pub pos: V2,
    pub id: String,
    pub alive: bool,
    pub died_at: f64,
    restore_s: Option<f64>,
    detected_until: f64,
    radar: bool,
    /// Ships: patrol route, distance along it and speed (m/s).
    pub route: Option<(Path, f64, f64)>,
}

pub struct SimConvoy {
    pub team: u8,
    pub path: Path,
    pub s: f64,
    pub speed: f64,
    pub n: u32,
    pub supply: bool,
    pub from_site: usize,
    pub to_site: usize,
    vehicle: String,
    detected_until: f64,
}

impl SimConvoy {
    pub fn head(&self) -> V2 {
        self.path.at(self.s).0
    }
}

struct Depot {
    team: u8,
    site: usize,
    stock: f64,
    next_armor: f64,
    supply_to: Vec<usize>,
    armor_to: Vec<usize>,
    supply_vehicle: String,
}

struct Plane {
    team: u8,
    centre: V2,
    radius: f64,
    phase: f64,
    rate: f64,
}

impl Plane {
    fn at(&self, t: f64) -> (V2, V2) {
        let a = self.phase + t * self.rate;
        (
            self.centre.add(V2(a.cos(), a.sin()).scale(self.radius)),
            V2(-a.sin(), a.cos()).scale(self.rate.signum()),
        )
    }
}

pub struct Sim {
    pack: Arc<Pack>,
    pub level: usize,
    pub variant: usize,
    pub era: usize,
    pub my_team: u8,
    pub t: f64,
    rng: Rng,
    pub units: Vec<SimUnit>,
    pub convoys: Vec<SimConvoy>,
    depots: Vec<Depot>,
    graph: RoadGraph,
    planes: Vec<Plane>,
    player_path: Path,
    player_s: f64,
    hud: Vec<(i64, String, f64)>,
    next_kill: f64,
    vehicle: String,
    /// Battle clock at the moment the app "joined".
    pub generation: i64,
    /// Chance that a kill hits a unit currently on the player's map.
    pub visible_kill_bias: f64,
    pub kill_every: (f64, f64),
    /// Paint airfields in a neutral colour instead of the team colours.
    pub neutral_airfields: bool,
    /// Shift every ground unit's icon by this many metres in one direction
    /// (plus a little scatter), as if the map were slightly off.
    pub position_error_m: f64,
    /// Multiplies every sensor range (lower = fewer enemies on the map).
    pub sensor_scale: f64,
    /// Add point-of-interest and aiming-point marks to the map.
    pub player_marks: bool,
}

impl Sim {
    pub fn new(
        pack: Arc<Pack>,
        level: usize,
        variant: usize,
        era: usize,
        my_team: u8,
        seed: u64,
    ) -> Sim {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let lvl = &pack.levels[level];
        let var = &lvl.variants[variant];
        let mut units = Vec::new();
        for (si, site) in var.sites.iter().enumerate() {
            let ship_route = site
                .route
                .as_ref()
                .filter(|r| site.cat == "ship" && r.pts.len() >= 4);
            if site.moves.is_some() && ship_route.is_none() {
                continue;
            }
            for (ui, placement) in site.units.iter().enumerate() {
                if placement.e >> era & 1 == 0 {
                    continue;
                }
                let route = ship_route.map(|r| {
                    let mut points: Vec<V2> =
                        r.pts.chunks_exact(2).map(|c| V2(c[0], c[1])).collect();
                    if r.closed {
                        points.push(points[0]);
                    }
                    // ships sail a bit below their top speed
                    let kph = pack
                        .core
                        .units
                        .get(&placement.u)
                        .and_then(|u| u.kph)
                        .unwrap_or(40.0);
                    (Path::new(points), 0.0, kph / 3.6 * 0.9)
                });
                let restore_s = match placement.l.as_deref() {
                    Some("restore") => Some(pack.core.template_restore.restore_seconds),
                    Some("repair_with_site") => {
                        Some(pack.core.template_restore.depot_repair_seconds)
                    }
                    _ => None,
                };
                units.push(SimUnit {
                    site: si as u16,
                    slot: ui as u16,
                    team: site.team,
                    pos: placement.p,
                    id: placement.u.clone(),
                    alive: true,
                    died_at: 0.0,
                    restore_s: restore_s.filter(|s| *s > 0.0),
                    detected_until: f64::NEG_INFINITY,
                    radar: site.cat == "ew_radar",
                    route,
                });
            }
        }
        let rules = &pack.core.rules.resource_factory;
        let convoy_ids = pack.core.convoy_unit_ids();
        let truck = |team: u8| {
            let prefix = if team == 1 { "ussr" } else { "us_" };
            convoy_ids
                .iter()
                .find(|id| id.contains("carrier") && id.contains(prefix))
                .cloned()
                .unwrap_or_else(|| "truck".into())
        };
        let mut depots: Vec<Depot> = Vec::new();
        for (si, site) in var.sites.iter().enumerate() {
            if !site.is_depot() {
                continue;
            }
            let links = |links: &[(String, String, u32)]| -> Vec<usize> {
                links
                    .iter()
                    .filter(|(from, _, _)| *from == site.id)
                    .filter_map(|(_, to, _)| var.site_index(to))
                    .collect()
            };
            let low = rules.initial_fraction.first().copied().unwrap_or(0.2);
            let high = rules.initial_fraction.get(1).copied().unwrap_or(0.4);
            depots.push(Depot {
                team: site.team,
                site: si,
                stock: rules.capacity * rng.range(low, high),
                next_armor: rng.range(60.0, 300.0),
                supply_to: links(&var.supply_links),
                armor_to: links(&var.armor_links),
                supply_vehicle: truck(site.team),
            });
        }
        let centre = |team: u8| {
            let mine: Vec<V2> = var
                .sites
                .iter()
                .filter(|s| s.team == team && s.cat == "strongpoint")
                .map(|s| s.p)
                .collect();
            mine.iter()
                .fold(V2(0.0, 0.0), |a, p| a.add(*p))
                .scale(1.0 / mine.len().max(1) as f64)
        };
        let (home, away) = (centre(my_team), centre(3 - my_team));
        let mut planes = Vec::new();
        for i in 0..4 {
            let t = 0.25 + 0.2 * i as f64;
            planes.push(Plane {
                team: my_team,
                centre: home.lerp(away, t),
                radius: rng.range(5000.0, 9000.0),
                phase: rng.range(0.0, 6.28),
                rate: rng.range(0.02, 0.04),
            });
            planes.push(Plane {
                team: 3 - my_team,
                centre: away.lerp(home, t * 0.8),
                radius: rng.range(5000.0, 9000.0),
                phase: rng.range(0.0, 6.28),
                rate: -rng.range(0.02, 0.04),
            });
        }
        // The player flies a loop over the enemy's depots and strongpoints.
        let start = var
            .sites
            .iter()
            .find(|s| s.team == my_team && s.cat == "airfield")
            .map_or(home, |s| s.p);
        let mut stops: Vec<V2> = var
            .sites
            .iter()
            .filter(|s| s.team != my_team && (s.is_depot() || s.cat == "strongpoint"))
            .map(|s| s.p)
            .collect();
        stops.sort_by(|a, b| a.dist(start).partial_cmp(&b.dist(start)).unwrap());
        stops.truncate(8);
        let mut loop_points = vec![start];
        loop_points.extend(stops.iter().copied());
        loop_points.extend(stops.iter().rev().skip(1).copied());
        loop_points.push(start);
        let eras = &pack.core.eras;
        let mut candidates: Vec<&String> = pack
            .core
            .vehicles
            .iter()
            .filter(|(_, v)| {
                v.2 == "fighter" && v.0 >= eras[era].rank[0] && v.0 <= eras[era].rank[1]
            })
            // team 1 flies USSR and Chinese aircraft, team 2 everything else
            .filter(|(_, v)| matches!(v.1.as_str(), "ussr" | "china") == (my_team == 1))
            .map(|(id, _)| id)
            .collect();
        candidates.sort();
        let vehicle = candidates
            .first()
            .map_or_else(|| "f_16a_block_10".to_string(), |id| (*id).clone());
        let graph = RoadGraph::build(&lvl.road_file);
        Sim {
            level,
            variant,
            era,
            my_team,
            t: 0.0,
            units,
            convoys: Vec::new(),
            depots,
            graph,
            planes,
            player_path: Path::new(loop_points),
            player_s: 0.0,
            hud: Vec::new(),
            next_kill: rng.range(10.0, 25.0),
            vehicle,
            generation: 1 + (seed % 1000) as i64,
            visible_kill_bias: 0.75,
            kill_every: (14.0, 34.0),
            neutral_airfields: false,
            position_error_m: 0.0,
            sensor_scale: 1.0,
            player_marks: true,
            rng,
            pack,
        }
    }

    pub fn player(&self) -> (V2, V2) {
        self.player_path
            .at(self.player_s % self.player_path.length().max(1.0))
    }

    fn sensors(&self) -> Vec<(V2, f64, u8)> {
        let mut sensors: Vec<(V2, f64, u8)> = vec![(self.player().0, 20_000.0, self.my_team)];
        for plane in &self.planes {
            sensors.push((plane.at(self.t).0, 15_000.0, plane.team));
        }
        for unit in self.units.iter().filter(|u| u.alive && u.radar) {
            sensors.push((unit.pos, 45_000.0, unit.team));
        }
        sensors
    }

    fn say(&mut self, text: String) {
        let id = self.hud.len() as i64 + 1;
        self.hud.push((id, text, self.t.floor()));
    }

    fn unit_name(&self, id: &str) -> String {
        self.pack
            .core
            .units
            .get(id)
            .map_or(id.to_string(), |u| u.short().to_string())
    }

    pub fn step(&mut self, dt: f64) {
        self.t += dt;
        self.player_s += 280.0 * dt;
        let rules = self.pack.core.rules.resource_factory.clone();
        let var = &self.pack.levels[self.level].variants[self.variant];

        // depots produce and dispatch
        for di in 0..self.depots.len() {
            let site = &var.sites[self.depots[di].site];
            if site.cat != "assembly_area" && !self.depots[di].supply_to.is_empty() {
                self.depots[di].stock =
                    (self.depots[di].stock + rules.production_per_second * dt).min(rules.capacity);
                let on_road = self
                    .convoys
                    .iter()
                    .filter(|c| c.supply && c.from_site == self.depots[di].site)
                    .count();
                if self.depots[di].stock >= rules.dispatch_threshold && on_road < 2 {
                    self.depots[di].stock -= rules.dispatch_threshold;
                    let to = self.depots[di].supply_to
                        [self.rng.pick(self.depots[di].supply_to.len().min(3))];
                    let path = self.graph.route(site.p, var.sites[to].p);
                    let speed = rules.travel_speed.max(10.0) * self.rng.range(0.8, 0.92);
                    self.convoys.push(SimConvoy {
                        team: self.depots[di].team,
                        path,
                        s: 0.0,
                        speed,
                        n: rules.column_vehicle_count.max(3),
                        supply: true,
                        from_site: self.depots[di].site,
                        to_site: to,
                        vehicle: self.depots[di].supply_vehicle.clone(),
                        detected_until: f64::NEG_INFINITY,
                    });
                }
            }
            if site.cat == "assembly_area"
                && !self.depots[di].armor_to.is_empty()
                && self.t >= self.depots[di].next_armor
            {
                self.depots[di].next_armor = self.t + self.rng.range(240.0, 520.0);
                let to =
                    self.depots[di].armor_to[self.rng.pick(self.depots[di].armor_to.len().min(3))];
                let path = self.graph.route(site.p, var.sites[to].p);
                let speed = rules.travel_speed.max(10.0) * self.rng.range(0.75, 0.9);
                self.convoys.push(SimConvoy {
                    team: self.depots[di].team,
                    path,
                    s: 0.0,
                    speed,
                    n: 5,
                    supply: false,
                    from_site: self.depots[di].site,
                    to_site: to,
                    vehicle: self.depots[di].supply_vehicle.clone(),
                    detected_until: f64::NEG_INFINITY,
                });
            }
        }
        for convoy in &mut self.convoys {
            convoy.s += convoy.speed * dt;
        }
        for unit in &mut self.units {
            if let Some((path, s, speed)) = &mut unit.route {
                *s = (*s + *speed * dt).rem_euclid(path.length().max(1.0));
                unit.pos = path.at(*s).0;
            }
        }
        // An armoured column reaching a site whose defenders are all gone takes it.
        let arrived: Vec<(usize, u8)> = self
            .convoys
            .iter()
            .filter(|c| !c.supply && c.n > 0 && c.s >= c.path.length())
            .map(|c| (c.to_site, c.team))
            .collect();
        for (site, team) in arrived {
            let defenders: Vec<&SimUnit> = self
                .units
                .iter()
                .filter(|u| u.site as usize == site)
                .collect();
            if !defenders.is_empty() && defenders.iter().all(|u| !u.alive && u.team != team) {
                self.capture_site(site, team);
            }
        }
        self.convoys.retain(|c| c.s < c.path.length() && c.n > 0);

        // detection
        let sensors = self.sensors();
        let now = self.t;
        let scale = self.sensor_scale;
        let seen_by = |p: V2, team: u8| {
            sensors
                .iter()
                .any(|(sp, range, st)| *st != team && sp.dist(p) <= *range * scale)
        };
        for unit in &mut self.units {
            if unit.alive && seen_by(unit.pos, unit.team) {
                unit.detected_until = now + DETECT_LINGER_S;
            }
        }
        for convoy in &mut self.convoys {
            if seen_by(convoy.path.at(convoy.s).0, convoy.team) {
                convoy.detected_until = now + DETECT_LINGER_S;
            }
        }

        // restores
        for unit in &mut self.units {
            if !unit.alive
                && unit
                    .restore_s
                    .is_some_and(|after| now - unit.died_at >= after)
            {
                unit.alive = true;
            }
        }

        // kills
        if self.t >= self.next_kill {
            self.next_kill = self.t + self.rng.range(self.kill_every.0, self.kill_every.1);
            let roll = self.rng.next();
            let victim_team = if roll < 0.8 {
                3 - self.my_team
            } else {
                self.my_team
            };
            let enemy = 3 - self.my_team;
            if roll > 0.93 {
                if let Some(ci) = (0..self.convoys.len()).find(|&i| {
                    self.convoys[i].team == enemy && self.convoys[i].detected_until > now
                }) {
                    self.convoys[ci].n -= 1;
                    let name = self.unit_name(&self.convoys[ci].vehicle.clone());
                    let tag = self.rng.pick(90);
                    self.say(format!("Hunter_{tag} (F-16C) destroyed {name}"));
                    return;
                }
            }
            let want_visible =
                victim_team == self.my_team || self.rng.next() < self.visible_kill_bias;
            let pool: Vec<usize> = (0..self.units.len())
                .filter(|&i| {
                    let u = &self.units[i];
                    u.alive
                        && u.team == victim_team
                        && (victim_team == self.my_team || (u.detected_until > now) == want_visible)
                })
                .collect();
            if !pool.is_empty() {
                let index = pool[self.rng.pick(pool.len())];
                self.units[index].alive = false;
                self.units[index].died_at = now;
                self.units[index].detected_until = f64::NEG_INFINITY;
                let name = self.unit_name(&self.units[index].id.clone());
                let killer = if victim_team == self.my_team {
                    "Raider"
                } else {
                    "Hunter"
                };
                let tag = self.rng.pick(90);
                self.say(format!("{killer}_{tag} (F-16C) destroyed {name}"));
            }
        }
    }

    /// Makes the player fly a loop through these points instead.
    pub fn set_player_route(&mut self, mut points: Vec<V2>) {
        if points.len() >= 2 {
            points.push(points[0]);
            self.player_path = Path::new(points);
            self.player_s = 0.0;
        }
    }

    /// Destroys one unit and writes the kill-feed line, as the game does.
    pub fn destroy_unit(&mut self, index: usize) {
        let now = self.t;
        let unit = &mut self.units[index];
        unit.alive = false;
        unit.died_at = now;
        unit.detected_until = f64::NEG_INFINITY;
        let name = self.unit_name(&self.units[index].id.clone());
        let tag = self.rng.pick(90);
        self.say(format!("Hunter_{tag} (F-16C) destroyed {name}"));
    }

    /// A site changes hands: its positions are now held by `team`'s vehicles.
    pub fn capture_site(&mut self, site: usize, team: u8) {
        let now = self.t;
        for unit in self.units.iter_mut().filter(|u| u.site as usize == site) {
            unit.team = team;
            unit.alive = true;
            unit.died_at = now;
            unit.detected_until = f64::NEG_INFINITY;
        }
    }

    /// Answers one page of the game's local web interface.
    pub fn page(&self, path: &str) -> Option<String> {
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        let lvl = &self.pack.levels[self.level];
        let var = &lvl.variants[self.variant];
        let frame = crate::geo::MapFrame::from_bounds(lvl.bounds);
        let colour = |team: u8| if team == self.my_team { ALLY } else { ENEMY };
        let rgb = |team: u8| {
            if team == self.my_team {
                json!([23, 77, 255])
            } else {
                json!([250, 12, 0])
            }
        };
        match route {
            "/map_info.json" => Some(
                json!({
                    "valid": true,
                    "map_min": [lvl.bounds[0], lvl.bounds[1]],
                    "map_max": [lvl.bounds[2], lvl.bounds[3]],
                    "grid_zero": [lvl.bounds[0], lvl.bounds[3]],
                    "grid_steps": [(lvl.bounds[2] - lvl.bounds[0]) / 8.0, (lvl.bounds[3] - lvl.bounds[1]) / 8.0],
                    "grid_size": [lvl.bounds[2] - lvl.bounds[0], lvl.bounds[3] - lvl.bounds[1]],
                    "hud_type": 0,
                    "map_generation": self.generation,
                })
                .to_string(),
            ),
            "/map_obj.json" => {
                let mut objects = Vec::new();
                for site in &var.sites {
                    for runway in &site.rw {
                        let (sx, sy) = frame.to_norm(V2(runway[0], runway[1]));
                        let (ex, ey) = frame.to_norm(V2(runway[2], runway[3]));
                        let (c, c3) = if self.neutral_airfields { ("#c8c8c8", json!([200, 200, 200])) } else { (colour(site.team), rgb(site.team)) };
                        objects.push(json!({"type":"airfield","color":c,"color[]":c3,"blink":0,"icon":"none","icon_bg":"none","sx":sx,"sy":sy,"ex":ex,"ey":ey}));
                    }
                }
                for unit in &self.units {
                    if !unit.alive || (unit.team != self.my_team && unit.detected_until <= self.t) {
                        continue;
                    }
                    // a repeatable per-unit offset (ships are where they are)
                    let wobble = (unit.site as f64 * 12.9898 + unit.slot as f64 * 78.233).sin() * 43758.5453;
                    let angle = (wobble - wobble.floor()) * std::f64::consts::TAU;
                    let error = if unit.route.is_some() { 0.0 } else { self.position_error_m };
                    let shift = V2(0.87, 0.5).scale(error).add(V2(angle.cos(), angle.sin()).scale(error * 0.15));
                    let shifted = unit.pos.add(shift);
                    let (x, y) = frame.to_norm(shifted);
                    let icon = self.pack.core.units.get(&unit.id).map_or("Tank", |u| u.icon.as_str());
                    objects.push(json!({"type":"ground_model","color":colour(unit.team),"color[]":rgb(unit.team),"blink":0,"icon":icon,"icon_bg":"none","x":x,"y":y}));
                }
                for convoy in &self.convoys {
                    if convoy.team != self.my_team && convoy.detected_until <= self.t {
                        continue;
                    }
                    for i in 0..convoy.n {
                        let s = convoy.s - i as f64 * 25.0;
                        if s < 0.0 {
                            continue;
                        }
                        let (x, y) = frame.to_norm(convoy.path.at(s).0);
                        objects.push(json!({"type":"ground_model","color":colour(convoy.team),"color[]":rgb(convoy.team),"blink":0,"icon":"Wheeled","icon_bg":"none","x":x,"y":y}));
                    }
                }
                let (player, heading) = self.player();
                for plane in &self.planes {
                    let (p, dir) = plane.at(self.t);
                    if plane.team != self.my_team && p.dist(player) > 35_000.0 {
                        continue;
                    }
                    let (x, y) = frame.to_norm(p);
                    objects.push(json!({"type":"aircraft","color":colour(plane.team),"color[]":rgb(plane.team),"blink":0,"icon":"Fighter","icon_bg":"none","x":x,"y":y,"dx":dir.0,"dy":-dir.1}));
                }
                if self.player_marks {
                    let enemy_sites: Vec<V2> = var.sites.iter().filter(|s| s.team != self.my_team && s.is_air_defence()).map(|s| s.p).collect();
                    if let Some(first) = enemy_sites.first() {
                        let (x, y) = frame.to_norm(*first);
                        objects.push(json!({"type":"point_of_interest","color":"#ff0000","color[]":[255,0,0],"blink":0,"icon":"none","icon_bg":"none","x":x,"y":y}));
                    }
                    for site in enemy_sites.iter().skip(1).take(4) {
                        let (x, y) = frame.to_norm(*site);
                        objects.push(json!({"type":"aiming_point","color":"#ffd200","color[]":[255,210,0],"blink":0,"icon":"none","icon_bg":"none","x":x,"y":y}));
                    }
                }
                let (x, y) = frame.to_norm(player);
                objects.push(json!({"type":"aircraft","color":"#faC81E","color[]":[250,200,30],"blink":0,"icon":"Player","icon_bg":"none","x":x,"y":y,"dx":heading.0,"dy":-heading.1}));
                Some(serde_json::Value::Array(objects).to_string())
            }
            "/indicators" => Some(json!({"valid": true, "army": "air", "type": self.vehicle}).to_string()),
            "/state" => Some(json!({"valid": true, "H, m": 6200, "TAS, km/h": 1008, "IAS, km/h": 760}).to_string()),
            "/mission.json" => Some(json!({"objectives": [], "status": "running"}).to_string()),
            "/hudmsg" => {
                let last: i64 = query
                    .split('&')
                    .find_map(|part| part.strip_prefix("lastDmg="))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let damage: Vec<_> = self
                    .hud
                    .iter()
                    .filter(|(id, _, _)| *id > last)
                    .map(|(id, msg, time)| json!({"id": id, "msg": msg, "sender": "", "enemy": false, "mode": "", "time": time}))
                    .collect();
                Some(json!({"events": [], "damage": damage}).to_string())
            }
            _ => None,
        }
    }

    /// Polls every page once, the way the real link does.
    pub fn frame(&self, last_hud_id: i64) -> Frame {
        Frame {
            reachable: true,
            map_info: self
                .page("/map_info.json")
                .and_then(|t| telemetry::parse_map_info(&t)),
            objects: self
                .page("/map_obj.json")
                .and_then(|t| telemetry::parse_map_obj(&t)),
            vehicle: self
                .page("/indicators")
                .and_then(|t| telemetry::parse_indicators(&t)),
            flight: self.page("/state").and_then(|t| telemetry::parse_state(&t)),
            hud: self
                .page(&format!("/hudmsg?lastEvt=0&lastDmg={last_hud_id}"))
                .and_then(|t| telemetry::parse_hudmsg(&t))
                .unwrap_or_default(),
            mission_status: self
                .page("/mission.json")
                .and_then(|t| telemetry::parse_mission(&t)),
        }
    }

    pub fn last_hud_id(&self) -> i64 {
        self.hud.len() as i64
    }
}
