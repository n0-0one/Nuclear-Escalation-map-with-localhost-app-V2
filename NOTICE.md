# Credits and notices

**Nuclear Thunder Live** continues two earlier projects:

- [Nuclear Thunder Interactive Map](https://github.com/majorlier/nuclear-thunder-interactive-map) by **majorlier** - the original interactive map. This app uses data that project produces: unit placements, mission rules and unit ranges extracted from the datamine, plus terrain heights, road networks, navigation meshes and tactical map pictures extracted from the game client. Only data is used; the map interface here is new.
- [Nuclear-Escalation-map-with-localhost-app](https://github.com/n0-0one/Nuclear-Escalation-map-with-localhost-app) - the first live version, whose field notes on the game's local web interface shaped this one.

Game data comes from the public [War Thunder Datamine](https://github.com/gszabi99/War-Thunder-Datamine) by gszabi99. The exact revisions used are recorded in `data/manifest.json`.

War Thunder, its maps, vehicle names, vehicle artwork and all extracted game data belong to Gaijin Entertainment. This is an unofficial fan project. It is not affiliated with or endorsed by Gaijin Entertainment.

## Software

- [Tauri](https://tauri.app) - MIT or Apache-2.0
- [Preact](https://preactjs.com) and [@preact/signals](https://github.com/preactjs/signals) - MIT
- [serde](https://serde.rs), [ureq](https://github.com/algesten/ureq), [tiny_http](https://github.com/tiny-http/tiny-http), [sha2](https://github.com/RustCrypto/hashes), [native-tls](https://github.com/sfackler/rust-native-tls) - MIT or Apache-2.0
- Fonts: [Barlow Semi Condensed](https://github.com/jpt/barlow) and [JetBrains Mono](https://www.jetbrains.com/lp/mono/) - SIL Open Font License 1.1

Exact versions are pinned in `Cargo.lock` and `ui/package-lock.json`.
