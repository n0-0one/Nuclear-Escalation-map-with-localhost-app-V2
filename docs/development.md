# Building from source

## Layout

| Folder | What it is |
|---|---|
| `crates/engine` | The engine (Rust): reads the game, recognises the battle, tracks units and columns. No user interface. Includes a battle simulator. |
| `desktop` | The Windows app shell (Tauri 2): window, mini map window, data updates. |
| `ui` | The interface (TypeScript, Preact, plain canvas). |
| `pipeline` | Builds the data pack from the datamine (Python). |
| `data` | The built data pack. |

## Requirements

Rust (stable), Node.js 22, and for the Windows app the Visual Studio C++ build tools and WebView2.

## Test

```bash
cargo test --release -p ntl-engine
```

This plays simulated battles on every map, side and BR bracket and checks the engine's picture against what really happened in the simulation.

## Run in a browser (no game needed)

```bash
cd ui && npm ci && npm run build && cd ..
cargo run --release -p ntl-engine --features dev --bin ntl-dev
```

Open `http://127.0.0.1:8777` and start the demo battle from Settings.

## Build the Windows app

```bash
cd ui && npm ci && npm run build && cd ../desktop
npx @tauri-apps/cli@2 build
```

The installer is written to `target/release/bundle/nsis/`. To publish a release, run the **Build and test** workflow from the Actions tab and type the version tag (for example `v2.0.1`) into the *release* box, or push a tag that starts with `v`. The release gets the installer, a portable ZIP and checksums.
