"""Facts read straight from the datamine that the mission extractor does not cover.

* which Nuclear Escalation missions exist (so a new map is picked up without
  editing a list), the level each one uses and that level's map bounds;
* every playable vehicle's battle rating, used to hint at the BR bracket;
* localized unit names and kill-log verbs, used to read the in-game kill feed
  in whatever language the game runs in;
* nuclear bomb yields.
"""

from __future__ import annotations

import csv
import io
import re

from datamine import Datamine

ZERO_WIDTH = dict.fromkeys(map(ord, "​‌‍﻿"), None)
LANG_SKIP = {"Comments", "max_chars"}


def read_csv(dm: Datamine, path: str) -> tuple[list[str], list[list[str]]]:
    text = dm.read_text(path).translate(ZERO_WIDTH)
    rows = list(csv.reader(io.StringIO(text), delimiter=";", quotechar='"'))
    header = [re.sub(r"[<>]", "", cell).split("|")[0] for cell in rows[0]]
    return header, rows[1:]


def discover_missions(dm: Datamine, config: dict) -> list[dict]:
    """Every mission file in the event folder, grouped later by level."""
    suffix = config["mission_suffix"]
    missions = []
    for path in sorted(dm.ls(config["mission_dir"])):
        name = path.rsplit("/", 1)[-1]
        if not name.endswith(suffix):
            continue
        document = dm.read_json(path)
        settings = document.get("mission_settings", {}).get("mission", {})
        level = str(settings.get("level", "")).rsplit("/", 1)[-1].removesuffix(".bin")
        stem = name[: -len(suffix)]
        mirror = stem.endswith("_mirror")
        if not level:
            level = stem.removesuffix("_mirror")
        missions.append(
            {
                "mission": name.removesuffix(".blkx"),
                "path": path,
                "sha": dm.blob_sha(path),
                "level": level,
                "mirror": mirror,
                "settings": {
                    key: settings[key]
                    for key in (
                        "timeToLoseInstantDetection",
                        "isAlwaysShowCollectiveDetectedUnits",
                        "showAIKilllog",
                        "afterRespawnDelaySec",
                        "scoreLimit",
                        "timeLimit",
                        "nuclearEscalationStage1",
                        "nuclearEscalationStage2",
                        "nuclearEscalationStage3",
                    )
                    if key in settings
                },
            }
        )
    return missions


def level_meta(dm: Datamine, level: str) -> dict | None:
    path = f"aces.vromfs.bin_u/levels/{level}.blkx"
    if not dm.exists(path):
        return None
    document = dm.read_json(path)
    low, high = document.get("mapCoord0"), document.get("mapCoord1")
    if not (isinstance(low, list) and isinstance(high, list)):
        return None
    return {
        "bounds": [float(low[0]), float(low[1]), float(high[0]), float(high[1])],
        "water": float(document.get("water_level", 0.0)),
        "grid_step": document.get("gridSteps"),
        "sha": dm.blob_sha(path),
        "path": path,
    }


def level_names(dm: Datamine) -> dict[str, str]:
    try:
        header, rows = read_csv(dm, "lang.vromfs.bin_u/lang/missions_locations.csv")
    except Exception:
        return {}
    english = header.index("English") if "English" in header else 1
    names = {}
    for row in rows:
        if row and row[0].startswith("location/") and len(row) > english and row[english]:
            names[row[0].split("/", 1)[1]] = row[english]
    return names


COUNTRY = re.compile(r"^country_")
CLASS = re.compile(r"^exp_")


def vehicles(dm: Datamine, shop_names: dict[str, str]) -> dict[str, list]:
    """``{unit id: [economic rank, country, class, name]}`` for aircraft and tanks."""
    wpcost = dm.read_json("char.vromfs.bin_u/config/wpcost.blkx")
    table = {}
    for unit_id, entry in wpcost.items():
        if not isinstance(entry, dict):
            continue
        if entry.get("unitMoveType") not in ("air", "helicopter", "tank"):
            continue
        rank = entry.get("economicRankHistorical")
        if not isinstance(rank, int):
            continue
        table[unit_id] = [
            rank,
            COUNTRY.sub("", str(entry.get("country", ""))),
            CLASS.sub("", str(entry.get("unitClass", ""))),
            shop_names.get(unit_id, ""),
        ]
    return table


def unit_tables(dm: Datamine, wanted: set[str]) -> tuple[dict, dict, dict]:
    """Returns (english display names, hud name index, shop names for all units).

    ``wanted`` holds the event's unit ids. Their names in every language go
    into the hud index: ``{lowercase name: [unit ids]}``.
    """
    header, rows = read_csv(dm, "lang.vromfs.bin_u/lang/units.csv")
    languages = [(index, name) for index, name in enumerate(header) if index and name and name not in LANG_SKIP]
    english = next((index for index, name in languages if name == "English"), 1)
    wanted_keys = {unit_id.rsplit("/", 1)[-1]: unit_id for unit_id in wanted}

    display: dict[str, dict[str, str]] = {}
    hud: dict[str, set[str]] = {}
    shop: dict[str, str] = {}
    for row in rows:
        if len(row) <= english:
            continue
        key = row[0]
        match = re.match(r"^(.*)_(shop|0|1|2)$", key)
        if not match:
            continue
        base, kind = match.group(1), match.group(2)
        if kind == "shop" and row[english]:
            shop[base] = row[english]
        unit_id = wanted_keys.get(base)
        if unit_id is None:
            continue
        if row[english]:
            display.setdefault(unit_id, {})[kind] = row[english]
        if kind == "2":
            continue  # generic class label such as "SAM"; useless for matching
        for index, _language in languages:
            if index < len(row):
                name = row[index].strip().lower()
                if len(name) >= 2:
                    hud.setdefault(name, set()).add(unit_id)
    names = {}
    for unit_id, kinds in display.items():
        full = kinds.get("0") or kinds.get("shop") or kinds.get("1")
        short = kinds.get("1") or kinds.get("shop") or full
        names[unit_id] = [full, short]
    return names, {name: sorted(ids) for name, ids in hud.items()}, shop


KILL_KEYS = {
    "ground": ["NET_UNIT_KILLED_GM"],
    "air": ["NET_UNIT_KILLED_FM"],
    "passive": ["NET_UNIT_KILLED_BY_PLAYER", "NET_PLAYER_GM_KILLED_BY_UNIT"],
}


def kill_verbs(dm: Datamine) -> dict[str, list[str]]:
    """Kill-log verbs in every language, e.g. ``destroyed`` / ``shot down``."""
    found: dict[str, set[str]] = {kind: set() for kind in KILL_KEYS}
    wanted = {key: kind for kind, keys in KILL_KEYS.items() for key in keys}
    for name in ("ui.csv", "menu.csv", "menu_multiplayer.csv"):
        path = f"lang.vromfs.bin_u/lang/{name}"
        if not dm.exists(path):
            continue
        header, rows = read_csv(dm, path)
        for row in rows:
            kind = wanted.get(row[0]) if row else None
            if not kind:
                continue
            for index, language in enumerate(header):
                if index and language not in LANG_SKIP and index < len(row):
                    # Some languages wrap the names with two fragments
                    # separated by a literal "\t"; keep the longest one.
                    pieces = [piece.strip(" :") for piece in row[index].lower().split("\\t")]
                    verb = max(pieces, key=len, default="")
                    if len(verb) >= 2:
                        found[kind].add(verb)
    return {kind: sorted(verbs, key=lambda verb: (-len(verb), verb)) for kind, verbs in found.items()}


def nuclear_weapons(dm: Datamine, hints: list[str]) -> list[dict]:
    folder = "aces.vromfs.bin_u/gamedata/weapons/bombguns"
    weapons = []
    for path in dm.ls(folder):
        stem = path.rsplit("/", 1)[-1].removesuffix(".blkx")
        if not any(hint in stem for hint in hints):
            continue
        try:
            bomb = dm.read_json(path).get("bomb", {})
        except Exception:
            continue
        kilotons = bomb.get("yield") if isinstance(bomb, dict) else None
        if isinstance(kilotons, (int, float)) and kilotons > 0:
            weapons.append({"id": stem, "yield_kt": float(kilotons)})
    return sorted(weapons, key=lambda item: (item["yield_kt"], item["id"]))
