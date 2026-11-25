//! Parsing of the game's local web interface (`http://127.0.0.1:8111`).
//!
//! The game offers these read-only JSON pages while it runs. Nothing here
//! assumes a field exists: unknown or missing values are skipped, so a game
//! update that adds or drops a field does not break the app.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Aff {
    Player,
    Ally,
    Enemy,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Aircraft,
    Ground,
    Airfield,
    Zone,
    Other,
}

#[derive(Debug, Clone)]
pub struct MapObj {
    pub kind: Kind,
    /// The game's own type name, kept so new kinds of markers can be told apart.
    pub type_name: String,
    pub icon: String,
    pub rgb: [u8; 3],
    pub blink: bool,
    /// Normalized position (0..1, y down). For airfields: runway start.
    pub x: f64,
    pub y: f64,
    /// Heading vector, when the game reports one.
    pub dx: f64,
    pub dy: f64,
    /// Runway end, airfields only.
    pub ex: f64,
    pub ey: f64,
}

impl MapObj {
    pub fn is_player(&self) -> bool {
        self.icon.eq_ignore_ascii_case("player")
    }
}

/// Works out who an icon belongs to from its colour.
///
/// The game paints enemies red and friends blue (squad mates green) unless
/// the player changed the colours; `swapped` covers the common inverted setup.
pub fn affiliation(obj: &MapObj, swapped: bool) -> Aff {
    if obj.is_player() {
        return Aff::Player;
    }
    let [r, g, b] = obj.rgb.map(i32::from);
    let aff = if r > 150 && r > g + 60 && r > b + 60 {
        Aff::Enemy
    } else if (b > 120 && b > r + 40) || (g > 120 && g > r + 50) {
        Aff::Ally
    } else {
        Aff::Unknown
    };
    match (aff, swapped) {
        (Aff::Enemy, true) => Aff::Ally,
        (Aff::Ally, true) => Aff::Enemy,
        (other, _) => other,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapInfo {
    pub min: [f64; 2],
    pub max: [f64; 2],
    pub grid_zero: Option<[f64; 2]>,
    pub grid_steps: Option<[f64; 2]>,
    pub generation: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vehicle {
    /// Internal vehicle id, e.g. `f_16a_block_10`.
    pub id: String,
    /// "air" or "tank".
    pub army: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Flight {
    pub altitude_m: Option<f64>,
    pub tas_kmh: Option<f64>,
    pub ias_kmh: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HudLine {
    pub id: i64,
    pub text: String,
    /// Seconds since the battle started.
    pub time: Option<f64>,
}

/// One poll of the game. A `None` member means "no fresh answer this time",
/// which is different from "the game said there is nothing".
#[derive(Debug, Clone, Default)]
pub struct Frame {
    /// The web interface answered at all.
    pub reachable: bool,
    pub map_info: Option<Option<MapInfo>>,
    pub objects: Option<Vec<MapObj>>,
    pub vehicle: Option<Option<Vehicle>>,
    pub flight: Option<Flight>,
    pub hud: Vec<HudLine>,
    pub mission_status: Option<String>,
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn pair(v: &Value, key: &str) -> Option<[f64; 2]> {
    let a = v.get(key)?.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

fn rgb(v: &Value) -> [u8; 3] {
    if let Some(a) = v.get("color[]").and_then(Value::as_array) {
        if a.len() >= 3 {
            let c = |i: usize| a[i].as_f64().unwrap_or(0.0).clamp(0.0, 255.0) as u8;
            return [c(0), c(1), c(2)];
        }
    }
    if let Some(text) = v.get("color").and_then(Value::as_str) {
        let hex = text.trim_start_matches('#');
        if hex.len() >= 6 {
            let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
            return [c(0), c(2), c(4)];
        }
    }
    [0, 0, 0]
}

/// `/map_obj.json` - everything drawn on the player's tactical map.
pub fn parse_map_obj(text: &str) -> Option<Vec<MapObj>> {
    let value: Value = serde_json::from_str(text).ok()?;
    let items = value.as_array()?;
    let mut out = Vec::with_capacity(items.len().min(4096));
    for item in items.iter().take(4096) {
        let type_name = item.get("type").and_then(Value::as_str).unwrap_or("");
        let icon = item
            .get("icon")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let kind = match type_name {
            "aircraft" => Kind::Aircraft,
            "ground_model" => Kind::Ground,
            t if t.contains("ship") || t.contains("boat") => Kind::Ground,
            "airfield" => Kind::Airfield,
            t if t.contains("zone") || t.contains("point") || t.contains("respawn") => Kind::Zone,
            _ => Kind::Other,
        };
        let (x, y, ex, ey) = if kind == Kind::Airfield {
            match (
                num(item, "sx"),
                num(item, "sy"),
                num(item, "ex"),
                num(item, "ey"),
            ) {
                (Some(sx), Some(sy), Some(ex), Some(ey)) => (sx, sy, ex, ey),
                _ => continue,
            }
        } else {
            match (num(item, "x"), num(item, "y")) {
                (Some(x), Some(y)) => (x, y, 0.0, 0.0),
                _ => continue,
            }
        };
        out.push(MapObj {
            kind,
            type_name: type_name.chars().take(48).collect(),
            icon,
            rgb: rgb(item),
            blink: num(item, "blink").unwrap_or(0.0) != 0.0,
            x,
            y,
            dx: num(item, "dx").unwrap_or(0.0),
            dy: num(item, "dy").unwrap_or(0.0),
            ex,
            ey,
        });
    }
    Some(out)
}

/// `/map_info.json` - map bounds; `None` inside when no battle map is loaded.
pub fn parse_map_info(text: &str) -> Option<Option<MapInfo>> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("valid").and_then(Value::as_bool) != Some(true) {
        return Some(None);
    }
    let (Some(min), Some(max)) = (pair(&value, "map_min"), pair(&value, "map_max")) else {
        return Some(None);
    };
    if !(max[0] > min[0] && max[1] > min[1]) {
        return Some(None);
    }
    Some(Some(MapInfo {
        min,
        max,
        grid_zero: pair(&value, "grid_zero"),
        grid_steps: pair(&value, "grid_steps"),
        generation: value
            .get("map_generation")
            .and_then(Value::as_i64)
            .unwrap_or(0),
    }))
}

/// `/indicators` - which vehicle the player sits in.
pub fn parse_indicators(text: &str) -> Option<Option<Vehicle>> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("valid").and_then(Value::as_bool) != Some(true) {
        return Some(None);
    }
    let id = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if id.is_empty() {
        return Some(None);
    }
    Some(Some(Vehicle {
        id,
        army: value
            .get("army")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }))
}

/// `/state` - flight data. Keys carry their unit in the name ("H, m").
pub fn parse_state(text: &str) -> Option<Flight> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("valid").and_then(Value::as_bool) != Some(true) {
        return Some(Flight::default());
    }
    Some(Flight {
        altitude_m: num(&value, "H, m"),
        tas_kmh: num(&value, "TAS, km/h"),
        ias_kmh: num(&value, "IAS, km/h"),
    })
}

/// `/hudmsg?lastEvt=..&lastDmg=..` - the kill feed. Returns new lines.
pub fn parse_hudmsg(text: &str) -> Option<Vec<HudLine>> {
    let value: Value = serde_json::from_str(text).ok()?;
    let mut out = Vec::new();
    for line in value
        .get("damage")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(500)
    {
        let Some(msg) = line.get("msg").and_then(Value::as_str) else {
            continue;
        };
        out.push(HudLine {
            id: line.get("id").and_then(Value::as_i64).unwrap_or(0),
            text: msg.chars().take(400).collect(),
            time: num(line, "time"),
        });
    }
    Some(out)
}

/// `/mission.json` - "running", "success", "fail" or similar.
pub fn parse_mission(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    Some(
        value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_map_objects_and_skips_broken_ones() {
        let text = r##"[
          {"type":"airfield","color":"#174DFF","color[]":[23,77,255],"blink":0,"icon":"none","sx":0.1,"sy":0.2,"ex":0.11,"ey":0.21},
          {"type":"ground_model","color":"#fa0C00","color[]":[250,12,0],"blink":0,"icon":"Airdefence","x":0.5,"y":0.5},
          {"type":"aircraft","color":"#faC81E","color[]":[250,200,30],"blink":0,"icon":"Player","x":0.3,"y":0.4,"dx":0.6,"dy":-0.8},
          {"type":"ground_model","icon":"Tank"},
          {"type":"something_new","color":"#00ff00","x":0.9,"y":0.9}
        ]"##;
        let objects = parse_map_obj(text).unwrap();
        assert_eq!(objects.len(), 4);
        assert_eq!(objects[0].kind, Kind::Airfield);
        assert_eq!(affiliation(&objects[0], false), Aff::Ally);
        assert_eq!(affiliation(&objects[1], false), Aff::Enemy);
        assert_eq!(affiliation(&objects[1], true), Aff::Ally);
        assert_eq!(affiliation(&objects[2], false), Aff::Player);
        assert_eq!(objects[3].kind, Kind::Other);
    }

    #[test]
    fn colour_falls_back_to_hex() {
        let objects = parse_map_obj(
            r##"[{"type":"ground_model","color":"#fa0C00","icon":"x","x":0.1,"y":0.1}]"##,
        )
        .unwrap();
        assert_eq!(objects[0].rgb, [250, 12, 0]);
    }

    #[test]
    fn invalid_pages_are_not_errors() {
        assert_eq!(parse_map_info(r#"{"valid":false}"#), Some(None));
        assert_eq!(parse_indicators(r#"{"valid":false}"#), Some(None));
        assert!(parse_map_obj("not json").is_none());
        let info = parse_map_info(r#"{"valid":true,"map_min":[-65536,-65536],"map_max":[65536,65536],"map_generation":3,"grid_zero":[-65536,65536],"grid_steps":[9216,9216]}"#).unwrap().unwrap();
        assert_eq!(info.generation, 3);
        assert_eq!(info.grid_steps, Some([9216.0, 9216.0]));
    }

    #[test]
    fn reads_vehicle_flight_and_kill_feed() {
        let vehicle = parse_indicators(r#"{"valid":true,"army":"air","type":"F_16A_Block_10"}"#)
            .unwrap()
            .unwrap();
        assert_eq!(vehicle.id, "f_16a_block_10");
        let flight = parse_state(r#"{"valid":true,"H, m":5120,"TAS, km/h":930}"#).unwrap();
        assert_eq!(flight.altitude_m, Some(5120.0));
        let lines = parse_hudmsg(r#"{"events":[],"damage":[{"id":7,"msg":"Bot (F-16A) destroyed 2S6","sender":"","enemy":false,"mode":"","time":412}]}"#).unwrap();
        assert_eq!(
            lines[0],
            HudLine {
                id: 7,
                text: "Bot (F-16A) destroyed 2S6".into(),
                time: Some(412.0)
            }
        );
    }
}
