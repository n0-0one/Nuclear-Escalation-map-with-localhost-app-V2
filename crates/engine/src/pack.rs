//! The data pack: everything the app knows about the event, generated from
//! the public datamine by `pipeline/pack.py`.
//!
//! Every field is optional or defaulted where the game files could plausibly
//! change, so a pack built after a game update still loads.

use crate::geo::V2;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
pub struct Era {
    pub id: String,
    pub label: String,
    pub rank: [i32; 2],
    pub br: [f32; 2],
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UnitSpec {
    /// Display name.
    #[serde(default)]
    pub n: String,
    /// Short display name, when it differs.
    #[serde(default)]
    pub s: Option<String>,
    /// Icon the game uses for this unit on radar and the map.
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub det: Option<f64>,
    #[serde(default)]
    pub trk: Option<f64>,
    #[serde(default)]
    pub eng: Option<f64>,
    #[serde(default)]
    pub ai_eng: Option<f64>,
    #[serde(default)]
    pub gun: Option<f64>,
    #[serde(default)]
    pub kph: Option<f64>,
}

impl UnitSpec {
    /// Farthest distance at which this unit can hurt an aircraft.
    pub fn reach(&self) -> f64 {
        self.eng.or(self.ai_eng).or(self.gun).unwrap_or(0.0)
    }
    pub fn short(&self) -> &str {
        self.s.as_deref().unwrap_or(&self.n)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HudTables {
    #[serde(default)]
    pub verbs: HashMap<String, Vec<String>>,
    /// Lower-case unit name in any language -> unit ids.
    #[serde(default)]
    pub names: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LevelRef {
    pub id: String,
    pub name: String,
    pub bounds: [f64; 4],
    pub file: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ResourceRules {
    #[serde(default)]
    pub capacity: f64,
    #[serde(default)]
    pub initial_fraction: Vec<f64>,
    #[serde(default)]
    pub production_per_second: f64,
    #[serde(default)]
    pub dispatch_threshold: f64,
    #[serde(default)]
    pub column_vehicle_count: u32,
    #[serde(default)]
    pub travel_speed: f64,
    #[serde(default)]
    pub repair_seconds: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rules {
    #[serde(default)]
    pub resource_factory: ResourceRules,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TemplateRestore {
    #[serde(default)]
    pub restore_seconds: f64,
    #[serde(default)]
    pub depot_repair_seconds: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConvoyEntry {
    pub name: String,
    #[serde(default = "one")]
    pub count: u32,
}
fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize)]
pub struct Core {
    pub schema: u32,
    #[serde(default)]
    pub game_version: String,
    pub eras: Vec<Era>,
    pub units: HashMap<String, UnitSpec>,
    #[serde(default)]
    pub hud: HudTables,
    /// unit id -> [economic rank, country, class, name]
    #[serde(default)]
    pub vehicles: HashMap<String, (i32, String, String, String)>,
    #[serde(default)]
    pub rules: Rules,
    #[serde(default)]
    pub template_restore: TemplateRestore,
    /// Raw convoy compositions; walked generically because their shape
    /// follows the mission templates.
    #[serde(default)]
    pub convoys: serde_json::Value,
    pub levels: Vec<LevelRef>,
}

impl Core {
    /// Every unit id that appears in a convoy template, with its head count.
    pub fn convoy_unit_ids(&self) -> Vec<String> {
        fn walk(value: &serde_json::Value, out: &mut Vec<String>) {
            match value {
                serde_json::Value::Array(items) => {
                    for item in items {
                        if let Ok(entry) = serde_json::from_value::<ConvoyEntry>(item.clone()) {
                            out.push(entry.name);
                        } else {
                            walk(item, out);
                        }
                    }
                }
                serde_json::Value::Object(map) => map.values().for_each(|v| walk(v, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        walk(&self.convoys, &mut out);
        out.sort();
        out.dedup();
        out
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Placement {
    /// Unit id.
    pub u: String,
    pub p: V2,
    /// Bit `i` set = present in era `i`.
    pub e: u32,
    /// Role inside the site (launcher, tracker_radar, spaa, ...).
    #[serde(default)]
    pub r: String,
    /// How the unit comes back: "restore", "repair_with_site" or absent.
    #[serde(default)]
    pub l: Option<String>,
    /// Position is approximate.
    #[serde(default)]
    pub x: u8,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SeaRoute {
    pub pts: Vec<f64>,
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub speed_kph: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Site {
    pub id: String,
    pub team: u8,
    pub cat: String,
    pub p: V2,
    #[serde(default)]
    pub units: Vec<Placement>,
    /// Runways as [start x, start z, end x, end z, width].
    #[serde(default)]
    pub rw: Vec<[f64; 5]>,
    #[serde(default)]
    pub route: Option<SeaRoute>,
    /// Set when the unit does not stay where the mission file puts it.
    #[serde(default)]
    pub moves: Option<String>,
    #[serde(default)]
    pub cap: Option<f64>,
}

impl Site {
    pub fn is_depot(&self) -> bool {
        matches!(
            self.cat.as_str(),
            "assembly_area" | "fuel_factory" | "ammo_factory"
        )
    }
    pub fn is_air_defence(&self) -> bool {
        matches!(
            self.cat.as_str(),
            "long_sam" | "short_sam" | "sam_cover" | "spaa_cover"
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Variant {
    pub id: String,
    pub mission: String,
    #[serde(default)]
    pub mirror: bool,
    #[serde(default = "ten")]
    pub detect_loss_s: f64,
    pub sites: Vec<Site>,
    /// [factory site id, storage site id, rank by distance]
    #[serde(default)]
    pub supply_links: Vec<(String, String, u32)>,
    /// [assembly site id, target site id, rank by distance]
    #[serde(default)]
    pub armor_links: Vec<(String, String, u32)>,
}
fn ten() -> f64 {
    10.0
}

impl Variant {
    pub fn site_index(&self, id: &str) -> Option<usize> {
        self.sites.iter().position(|site| site.id == id)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Road {
    #[serde(default)]
    pub s: String,
    /// Flat x, y, z triples in metres.
    pub p: Vec<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NavMesh {
    /// Flat x, z pairs.
    pub v: Vec<f64>,
    /// Flat vertex index triples.
    pub t: Vec<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RoadFile {
    #[serde(default)]
    pub roads: Vec<Road>,
    #[serde(default)]
    pub navmesh: Option<NavMesh>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Level {
    pub id: String,
    pub name: String,
    /// min x, min z, max x, max z
    pub bounds: [f64; 4],
    pub variants: Vec<Variant>,
    #[serde(default)]
    pub roads: Option<String>,
    #[serde(skip)]
    pub road_file: RoadFile,
}

impl Level {
    pub fn variant(&self, id: &str) -> Option<&Variant> {
        self.variants.iter().find(|variant| variant.id == id)
    }
}

#[derive(Debug)]
pub struct Pack {
    pub dir: PathBuf,
    pub core: Core,
    pub levels: Vec<Level>,
}

#[derive(Debug)]
pub struct PackError(pub String);
impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PackError {}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, PackError> {
    let bytes = std::fs::read(path).map_err(|e| PackError(format!("{}: {e}", path.display())))?;
    serde_json::from_slice(&bytes).map_err(|e| PackError(format!("{}: {e}", path.display())))
}

impl Pack {
    /// Loads and sanity-checks a pack. A level that fails to parse is skipped
    /// rather than taking the whole app down.
    pub fn load(dir: &Path) -> Result<Pack, PackError> {
        let core: Core = read_json(&dir.join("core.json"))?;
        if core.schema != SCHEMA {
            return Err(PackError(format!(
                "data pack schema {} is not supported (expected {SCHEMA})",
                core.schema
            )));
        }
        if core.eras.is_empty() || core.eras.len() > 30 {
            return Err(PackError("data pack lists no BR brackets".into()));
        }
        let mut levels = Vec::new();
        for reference in &core.levels {
            // File names come from the pack; never let one climb out of it.
            if reference.file.contains("..") || reference.file.starts_with('/') {
                continue;
            }
            let Ok(mut level) = read_json::<Level>(&dir.join(&reference.file)) else {
                continue;
            };
            if level.variants.is_empty()
                || level.bounds[2] <= level.bounds[0]
                || level.bounds[3] <= level.bounds[1]
            {
                continue;
            }
            if let Some(name) = level.roads.clone() {
                if !name.contains("..") && !name.contains('/') {
                    if let Ok(file) = read_json::<RoadFile>(&dir.join("levels").join(name)) {
                        level.road_file = file;
                    }
                }
            }
            levels.push(level);
        }
        if levels.is_empty() {
            return Err(PackError("data pack contains no usable map".into()));
        }
        Ok(Pack {
            dir: dir.to_path_buf(),
            core,
            levels,
        })
    }

    pub fn level(&self, id: &str) -> Option<&Level> {
        self.levels.iter().find(|level| level.id == id)
    }

    pub fn era_index(&self, id: &str) -> Option<usize> {
        self.core.eras.iter().position(|era| era.id == id)
    }
}
