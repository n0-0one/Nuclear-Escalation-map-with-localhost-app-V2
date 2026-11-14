# Map data and updates

Everything the app knows about the event lives in the `data/` folder (about 13 MB for all three maps):

| File | Contents |
|---|---|
| `manifest.json` | Game version, source revisions and a SHA-256 for every file |
| `core.json` | BR brackets, unit ranges and names, vehicle battle ratings, kill-feed wording, mission rules |
| `levels/<map>.json` | Every site and unit position for both side layouts and all BR brackets |
| `levels/<map>.roads.json` | Road network used to route columns |
| `levels/<map>.webp`, `.height.webp` | Map picture and terrain heights |
| `units/*.webp` | Small unit pictures |

## Staying current after game updates

A scheduled job (`.github/workflows/data.yml`) runs every day:

1. reads the newest public datamine;
2. finds every Nuclear Escalation mission by itself - a new map or BR bracket needs no code change;
3. rebuilds the pack with `pipeline/update.py`;
4. runs the engine's battle tests against the new pack;
5. if anything changed, publishes it to the `data-feed` branch.

The app checks that branch shortly after it starts (and on demand in Settings). A download is verified file by file against the manifest's checksums and test-loaded before it replaces the working data, so a broken or interrupted update can never leave the app without a map. The pack is data only; nothing in it is run as code.

## Sources

- **Read directly from the datamine by this project:** the list of missions and maps, map bounds, mission settings, every vehicle's battle rating, unit names in all game languages, kill-feed wording, nuclear bomb yields.
- **Taken from majorlier's Nuclear Thunder Interactive Map:** unit placements, mission rules and unit ranges (which that project extracts from the datamine), and terrain, roads and map pictures (which exist only in the installed game, not in the datamine).

If the datamine's event files change before the placements catch up, the manifest lists them under `pending_game_changes` and the app shows a notice that positions may be out of date.

## A map the app has never seen

If you join a battle on a map that is not in the data yet, the app shows the game's own picture of that map with everything the game reports live on top, and suggests checking for a data update.

## Rebuilding by hand

```bash
python pipeline/update.py
```

Add `--assets` to also rebuild map pictures, terrain and roads. Requires Python 3.11+, Git and Pillow.
