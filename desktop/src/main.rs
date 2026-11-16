//! Nuclear Thunder Live - desktop shell.
//!
//! A thin window around the engine: it starts the engine, forwards its
//! snapshots to the interface, serves the data pack to the interface and
//! owns the small always-on-top mini map window.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod appupdate;
mod update;

use ntl_engine::host::Host;
use ntl_engine::Pack;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tauri::http::{Response, StatusCode};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

struct App {
    host: Host,
    pack_dir: PathBuf,
    /// Writable folder for downloaded data, settings and recordings.
    data_root: PathBuf,
    pack_source: &'static str,
}

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "json" => "application/json",
        "webp" => "image/webp",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn respond(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-cache")
        .body(body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

/// Serves one file of the data pack; never anything outside it.
fn serve_pack(pack_dir: &Path, uri_path: &str) -> Response<Vec<u8>> {
    let relative = Path::new(uri_path.trim_start_matches('/'));
    if relative.as_os_str().is_empty() || relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        return respond(StatusCode::BAD_REQUEST, "text/plain", Vec::new());
    }
    let file = pack_dir.join(relative);
    match std::fs::read(&file) {
        Ok(bytes) => respond(StatusCode::OK, mime(&file), bytes),
        Err(_) => respond(StatusCode::NOT_FOUND, "text/plain", Vec::new()),
    }
}

/// Everything the interface asks of the engine goes through here.
#[tauri::command]
fn engine(state: tauri::State<'_, App>, cmd: String, args: Value) -> Result<Value, String> {
    state.host.command(&cmd, args)
}

#[tauri::command]
fn app_info(state: tauri::State<'_, App>) -> Value {
    json!({ "version": env!("CARGO_PKG_VERSION"), "pack_source": state.pack_source, "portable": appupdate::is_portable() })
}

fn run_update(pack_dir: &Path, data_root: &Path) -> Result<String, String> {
    match update::update(pack_dir, data_root)? {
        update::Outcome::UpToDate => Ok("Map data is up to date.".into()),
        update::Outcome::NeedsNewerApp => Ok("Newer map data exists but needs a newer version of the app.".into()),
        update::Outcome::Downloaded { game_version } => Ok(format!("New map data for game version {game_version} was downloaded. Restart the app to use it.")),
    }
}

#[tauri::command]
async fn update_data(state: tauri::State<'_, App>) -> Result<String, String> {
    let (pack_dir, data_root) = (state.pack_dir.clone(), state.data_root.clone());
    tauri::async_runtime::spawn_blocking(move || run_update(&pack_dir, &data_root))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn app_update_check(window: tauri::WebviewWindow) -> Result<appupdate::Check, String> {
    if window.label() != "main" {
        return Err("Only the main window checks for updates.".into());
    }
    tauri::async_runtime::spawn_blocking(appupdate::check).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn app_update_install(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<String, String> {
    if window.label() != "main" {
        return Err("Only the main window installs updates.".into());
    }
    let (message, close) = tauri::async_runtime::spawn_blocking(appupdate::install).await.map_err(|e| e.to_string())??;
    if close {
        // Give the window a moment to show the message, then make way for the installer.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1200));
            app.exit(0);
        });
    }
    Ok(message)
}

/// Shows, hides or (un)locks the mini map window. Only the main window may ask.
#[tauri::command]
fn overlay(window: tauri::WebviewWindow, app: tauri::AppHandle, action: String) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the main window controls the mini map.".into());
    }
    let existing = app.get_webview_window("overlay");
    match (action.as_str(), existing) {
        ("show", Some(overlay)) => {
            overlay.set_ignore_cursor_events(false).map_err(|e| e.to_string())?;
            overlay.show().map_err(|e| e.to_string())
        }
        ("show", None) => WebviewWindowBuilder::new(&app, "overlay", WebviewUrl::App("index.html?overlay=1".into()))
            .title("Nuclear Thunder Live - mini map")
            .inner_size(300.0, 340.0)
            .min_inner_size(200.0, 230.0)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .build()
            .map(|_| ())
            .map_err(|e| e.to_string()),
        ("hide", Some(overlay)) => overlay.hide().map_err(|e| e.to_string()),
        ("locked", Some(overlay)) => overlay.set_ignore_cursor_events(true).map_err(|e| e.to_string()),
        ("unlocked", Some(overlay)) => overlay.set_ignore_cursor_events(false).map_err(|e| e.to_string()),
        (_, None) => Ok(()),
        _ => Err("Unknown mini map action.".into()),
    }
}

/// Picks the data pack to use: a downloaded one when it is valid and newer
/// than the one that shipped with the app, otherwise the shipped one.
fn choose_pack(bundled: &Path, data_root: &Path) -> Result<(Arc<Pack>, PathBuf, &'static str), String> {
    let downloaded = data_root.join("pack");
    let newer = match (update::read_manifest(&downloaded), update::read_manifest(bundled)) {
        (Some(d), Some(b)) => d.generated > b.generated,
        (Some(_), None) => true,
        _ => false,
    };
    if newer {
        if let Ok(pack) = Pack::load(&downloaded) {
            return Ok((Arc::new(pack), downloaded, "downloaded"));
        }
    }
    let pack = Pack::load(bundled).map_err(|e| e.to_string())?;
    Ok((Arc::new(pack), bundled.to_path_buf(), "built-in"))
}

fn main() {
    tauri::Builder::default()
        .register_uri_scheme_protocol("pack", |context, request| {
            match context.app_handle().try_state::<App>() {
                Some(state) => serve_pack(&state.pack_dir, request.uri().path()),
                None => respond(StatusCode::SERVICE_UNAVAILABLE, "text/plain", Vec::new()),
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let resources = app.path().resource_dir()?;
            // Development runs keep the pack at the repository root.
            let bundled = [resources.join("data"), PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../data")]
                .into_iter()
                .find(|dir| dir.join("core.json").exists())
                .ok_or("The app's map data is missing. Please reinstall.")?;
            let data_root = app.path().app_data_dir()?;
            let _ = std::fs::create_dir_all(&data_root);
            let (pack, pack_dir, pack_source) = choose_pack(&bundled, &data_root)?;
            let emitter = handle.clone();
            let host = Host::start(
                pack,
                Some(data_root.join("settings.json")),
                Some(data_root.join("recordings")),
                None,
                move |snapshot| {
                    let _ = emitter.emit("snapshot", snapshot);
                },
            );
            // Test hook: start in the demo battle (used by the automated start-up check).
            if let Ok(level) = std::env::var("NTL_DEMO") {
                let _ = host.command("demo_start", json!({ "level": level, "speed": 10.0 }));
            }
            app.manage(App { host, pack_dir: pack_dir.clone(), data_root: data_root.clone(), pack_source });

            // Look for newer map data shortly after start, without holding anything up.
            let notifier = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(6));
                if let Ok(update::Outcome::Downloaded { game_version }) = update::update(&pack_dir, &data_root) {
                    let _ = notifier.emit("data-updated", game_version);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window ends the app, mini map included.
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(state) = window.app_handle().try_state::<App>() {
                    state.host.stop();
                }
                window.app_handle().exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![engine, app_info, update_data, overlay, app_update_check, app_update_install])
        .run(tauri::generate_context!())
        .expect("the app window could not be created");
}
