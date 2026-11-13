//! Runs the engine: polls the game (or the built-in demo battle) on a
//! background thread and publishes snapshots. The desktop app and the
//! development server are both thin wrappers around this.

use crate::engine::{Engine, Settings};
use crate::link::Link;
use crate::pack::Pack;
use crate::sim::Sim;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
struct DemoArgs {
    level: String,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    era: Option<String>,
    #[serde(default = "team_one")]
    team: u8,
    #[serde(default = "speed_one")]
    speed: f64,
}
fn team_one() -> u8 {
    1
}
fn speed_one() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
struct MarkArgs {
    site: u16,
    #[serde(default)]
    slot: Option<u16>,
    /// true = destroyed, false = alive, absent = forget the manual mark.
    #[serde(default)]
    destroyed: Option<bool>,
}

struct Demo {
    sim: Sim,
    speed: f64,
    last_hud: i64,
}

struct Recorder {
    file: std::fs::File,
    path: PathBuf,
    bytes: u64,
    last_objects: f64,
}

struct Inner {
    engine: Engine,
    demo: Option<Demo>,
    link: Link,
    latest: Arc<String>,
    settings_path: Option<PathBuf>,
    record_dir: Option<PathBuf>,
    recorder: Option<Recorder>,
    game_base: Option<String>,
}

#[derive(Clone)]
pub struct Host {
    inner: Arc<Mutex<Inner>>,
    stop: Arc<AtomicBool>,
}

const RECORD_LIMIT: u64 = 150 * 1024 * 1024;

impl Host {
    /// `settings_path`: where settings are remembered. `record_dir`: where
    /// "record this battle" files go. `game_base` overrides the game address
    /// (tests and development only).
    pub fn start(
        pack: Arc<Pack>,
        settings_path: Option<PathBuf>,
        record_dir: Option<PathBuf>,
        game_base: Option<String>,
        on_snapshot: impl Fn(&str) + Send + 'static,
    ) -> Host {
        let mut engine = Engine::new(pack);
        if let Some(path) = &settings_path {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(settings) = serde_json::from_slice::<Settings>(&bytes) {
                    engine.set_settings(settings);
                }
            }
        }
        let link = game_base.as_deref().map_or_else(Link::new, Link::with_base);
        let latest = Arc::new(serde_json::to_string(&engine.snapshot()).unwrap_or_default());
        let host = Host {
            inner: Arc::new(Mutex::new(Inner { engine, demo: None, link, latest, settings_path, record_dir, recorder: None, game_base })),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let worker = host.clone();
        std::thread::Builder::new()
            .name("ntl-poll".into())
            .spawn(move || worker.run(on_snapshot))
            .expect("poll thread starts");
        host
    }

    fn run(&self, on_snapshot: impl Fn(&str)) {
        let started = Instant::now();
        let mut last_tick = Instant::now();
        while !self.stop.load(Ordering::Relaxed) {
            let now = started.elapsed().as_secs_f64();
            let dt = last_tick.elapsed().as_secs_f64().min(2.0);
            last_tick = Instant::now();
            let (text, pause) = {
                let mut guard = self.inner.lock().unwrap();
                let inner = &mut *guard;
                let frame = match &mut inner.demo {
                    Some(demo) => {
                        // Advance in small steps so fast-forward stays stable.
                        let mut remaining = dt * demo.speed;
                        while remaining > 0.0 {
                            let step = remaining.min(0.4);
                            demo.sim.step(step);
                            remaining -= step;
                        }
                        let frame = demo.sim.frame(demo.last_hud);
                        demo.last_hud = demo.sim.last_hud_id();
                        frame
                    }
                    None => {
                        let frame = inner.link.poll();
                        frame
                    }
                };
                if inner.demo.is_none() {
                    if let Some(recorder) = &mut inner.recorder {
                        recorder.write(now, &inner.link);
                        if recorder.bytes > RECORD_LIMIT {
                            inner.recorder = None;
                        }
                    }
                }
                inner.engine.ingest(&frame, now);
                let snapshot = inner.engine.snapshot();
                let mut value = serde_json::to_value(&snapshot).unwrap_or(Value::Null);
                if let Value::Object(map) = &mut value {
                    map.insert("demo".into(), json!(inner.demo.as_ref().map(|d| json!({"speed": d.speed, "t": d.sim.t.round()}))));
                    map.insert("recording".into(), json!(inner.recorder.as_ref().map(|r| r.path.display().to_string())));
                }
                let text = value.to_string();
                inner.latest = Arc::new(text.clone());
                let pause = match snapshot.link {
                    "battle" => 330,
                    "menu" => 1000,
                    _ => 2000,
                };
                (text, if inner.demo.is_some() { 330 } else { pause })
            };
            on_snapshot(&text);
            // Sleep in short slices so a command (demo start) takes effect quickly.
            let until = Instant::now() + Duration::from_millis(pause);
            while Instant::now() < until && !self.stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(30));
                if pause > 400 && self.inner.lock().unwrap().demo.is_some() {
                    break;
                }
            }
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn snapshot_json(&self) -> Arc<String> {
        self.inner.lock().unwrap().latest.clone()
    }

    pub fn pack(&self) -> Arc<Pack> {
        self.inner.lock().unwrap().engine.pack.clone()
    }

    /// Runs one command from the interface.
    pub fn command(&self, name: &str, args: Value) -> Result<Value, String> {
        let mut guard = self.inner.lock().unwrap();
        let inner = &mut *guard;
        match name {
            "snapshot" => serde_json::from_str(&inner.latest).map_err(|e| e.to_string()),
            "set_settings" => {
                let settings: Settings = serde_json::from_value(args).map_err(|e| e.to_string())?;
                inner.engine.set_settings(settings);
                if let Some(path) = &inner.settings_path {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(path, serde_json::to_vec_pretty(inner.engine.settings()).unwrap_or_default());
                }
                Ok(json!(inner.engine.settings()))
            }
            "mark" => {
                let mark: MarkArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                inner.engine.mark(mark.site, mark.slot, mark.destroyed);
                Ok(Value::Null)
            }
            "reset_battle" => {
                inner.engine.reset_battle();
                Ok(Value::Null)
            }
            "demo_start" => {
                let demo: DemoArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let pack = inner.engine.pack.clone();
                let level = pack.levels.iter().position(|l| l.id == demo.level).ok_or("Unknown map")?;
                let variant = demo.variant.as_deref().and_then(|v| pack.levels[level].variants.iter().position(|x| x.id == v)).unwrap_or(0);
                let era = demo.era.as_deref().and_then(|e| pack.era_index(e)).unwrap_or(pack.core.eras.len() - 1);
                let team = if demo.team == 2 { 2 } else { 1 };
                let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_secs());
                inner.engine.reset_battle();
                inner.demo = Some(Demo { sim: Sim::new(pack, level, variant, era, team, seed), speed: demo.speed.clamp(0.25, 20.0), last_hud: 0 });
                Ok(Value::Null)
            }
            "demo_speed" => {
                let speed = args.get("speed").and_then(Value::as_f64).ok_or("speed missing")?;
                if let Some(demo) = &mut inner.demo {
                    demo.speed = speed.clamp(0.25, 20.0);
                }
                Ok(Value::Null)
            }
            "demo_stop" => {
                if inner.demo.take().is_some() {
                    inner.engine.reset_battle();
                    inner.link = inner.game_base.as_deref().map_or_else(Link::new, Link::with_base);
                }
                Ok(Value::Null)
            }
            "record" => {
                let on = args.get("on").and_then(Value::as_bool).unwrap_or(false);
                if !on {
                    let path = inner.recorder.take().map(|r| r.path.display().to_string());
                    return Ok(json!(path));
                }
                let dir = inner.record_dir.clone().ok_or("Recording is not available here")?;
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let path = dir.join(format!("battle-{stamp}.jsonl"));
                let file = std::fs::OpenOptions::new().create_new(true).write(true).open(&path).map_err(|e| e.to_string())?;
                inner.recorder = Some(Recorder { file, path: path.clone(), bytes: 0, last_objects: f64::NEG_INFINITY });
                Ok(json!(path.display().to_string()))
            }
            "map_image" => {
                // The game's own picture of the current map, for maps without bundled art.
                if inner.demo.is_some() {
                    return Err("Not available in the demo".into());
                }
                let bytes = inner.link.map_image().ok_or("The game did not return a map picture")?;
                Ok(json!(bytes))
            }
            other => Err(format!("Unknown command: {other}")),
        }
    }
}

impl Recorder {
    /// Saves what the game reports, once a second, so a battle can be
    /// examined later. Nothing leaves the computer.
    fn write(&mut self, now: f64, link: &Link) {
        if now - self.last_objects < 1.0 {
            return;
        }
        self.last_objects = now;
        for path in ["/map_obj.json", "/map_info.json", "/indicators", "/state", "/hudmsg?lastEvt=0&lastDmg=0", "/mission.json"] {
            let Some(body) = link.raw(path) else { continue };
            let Ok(parsed) = serde_json::from_str::<Value>(&body) else { continue };
            let line = json!({"t": (now * 100.0).round() / 100.0, "path": path, "body": parsed}).to_string();
            if self.file.write_all(line.as_bytes()).and_then(|_| self.file.write_all(b"\n")).is_ok() {
                self.bytes += line.len() as u64 + 1;
            }
        }
    }
}
