//! Development server. Shows the interface in a normal browser and can play
//! a simulated battle, so the app can be worked on and tested without the
//! game or the desktop shell. It listens on 127.0.0.1 only and is not part
//! of the installed app.
//!
//!   ntl-dev [--data data] [--ui ui/dist] [--port 8777] [--game URL]
//!           [--serve-sim LEVEL] [--sim-speed N]
//!
//! `--serve-sim` makes this server answer the game's own pages under `/sim`,
//! so a second instance started with `--game http://127.0.0.1:PORT/sim`
//! exercises the real HTTP path to the "game".

use ntl_engine::host::Host;
use ntl_engine::sim::Sim;
use ntl_engine::Pack;
use std::sync::Mutex;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tiny_http::{Header, Method, Response, Server};

fn arg(name: &str, default: &str) -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned()).unwrap_or_else(|| default.to_string())
}

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "png" => "image/png",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// Resolves a request path inside `root`, refusing anything that climbs out.
fn safe_join(root: &Path, relative: &str) -> Option<PathBuf> {
    let relative = Path::new(relative.trim_start_matches('/'));
    if relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(root.join(relative))
}

fn main() {
    let data = PathBuf::from(arg("--data", "data"));
    let ui = PathBuf::from(arg("--ui", "ui/dist"));
    let port = arg("--port", "8777");
    let game = std::env::args().any(|a| a == "--game").then(|| arg("--game", ""));
    let pack = Arc::new(Pack::load(&data).unwrap_or_else(|e| {
        eprintln!("Cannot load the data pack from {}: {e}", data.display());
        std::process::exit(1);
    }));
    let sim: Option<Arc<Mutex<Sim>>> = std::env::args().any(|a| a == "--serve-sim").then(|| {
        let level = pack.levels.iter().position(|l| l.id == arg("--serve-sim", "")).unwrap_or(0);
        let speed: f64 = arg("--sim-speed", "1").parse().unwrap_or(1.0);
        let sim = Arc::new(Mutex::new(Sim::new(pack.clone(), level, 0, pack.core.eras.len() - 1, 1, 11)));
        let stepping = sim.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let mut left = 0.1 * speed;
            while left > 0.0 {
                stepping.lock().unwrap().step(left.min(0.4));
                left -= 0.4;
            }
        });
        sim
    });
    let host = Host::start(pack.clone(), None, Some(std::env::temp_dir().join("ntl-recordings")), game, |_| {});
    let server = Server::http(format!("127.0.0.1:{port}")).expect("port is free");
    println!("Nuclear Thunder Live development server: http://127.0.0.1:{port}");

    let header = |name: &str, value: &str| Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap();
    for mut request in server.incoming_requests() {
        let url = request.url().to_string();
        let path = url.split('?').next().unwrap_or("/").to_string();
        let response = if path == "/api/snapshot" {
            Response::from_string(host.snapshot_json().as_str()).with_header(header("Content-Type", "application/json")).boxed()
        } else if let (Some(page), Some(sim)) = (path.strip_prefix("/sim"), &sim) {
            if page == "/map.img" {
                let level = sim.lock().unwrap().level;
                let bytes = std::fs::read(data.join("levels").join(format!("{}.webp", pack.levels[level].id))).unwrap_or_default();
                Response::from_data(bytes).boxed()
            } else {
                let page = format!("{page}{}", url.find('?').map_or("", |i| &url[i..]));
                match sim.lock().unwrap().page(&page) {
                    Some(text) => Response::from_string(text).with_header(header("Content-Type", "application/json")).boxed(),
                    None => Response::from_string("{}").with_status_code(404).boxed(),
                }
            }
        } else if path == "/api/cmd" && *request.method() == Method::Post {
            let mut body = String::new();
            let _ = request.as_reader().take(1 << 20).read_to_string(&mut body);
            let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let name = parsed.get("cmd").and_then(|c| c.as_str()).unwrap_or("");
            let result = host.command(name, parsed.get("args").cloned().unwrap_or_default());
            let text = match result {
                Ok(value) => serde_json::json!({"ok": value}).to_string(),
                Err(error) => serde_json::json!({"error": error}).to_string(),
            };
            Response::from_string(text).with_header(header("Content-Type", "application/json")).boxed()
        } else {
            let file = if let Some(rest) = path.strip_prefix("/pack/") {
                safe_join(&data, rest)
            } else if path == "/" {
                Some(ui.join("index.html"))
            } else {
                safe_join(&ui, &path)
            };
            match file.and_then(|f| std::fs::read(&f).ok().map(|bytes| (f, bytes))) {
                Some((file, bytes)) => Response::from_data(bytes)
                    .with_header(header("Content-Type", mime(&file)))
                    .with_header(header("Cache-Control", "no-cache"))
                    .boxed(),
                None => Response::from_string("Not found").with_status_code(404).boxed(),
            }
        };
        let _ = request.respond(response);
    }
}
