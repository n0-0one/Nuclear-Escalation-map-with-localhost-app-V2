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


ICON_DIR = "atlases.vromfs.bin_u/gameuiskin"


def map_icons(dm: Datamine) -> dict[str, dict[str, str]]:
    """The game's map symbols in its three styles.

    ``{"wt": {type: svg}, "nato": {"ally_type" | "enemy_type": svg}, "ussr": {type: svg}}``.
    Each SVG draws its symbol in white (#fdfcff or #fff); the app paints that
    white in the team colour.
    """
    found: dict[str, dict[str, str]] = {"wt": {}, "nato": {}, "ussr": {}}
    for path in dm.ls(ICON_DIR):
        name = path.rsplit("/", 1)[-1]
        if not (name.startswith("def_") and name.endswith("_radar.svg")):
            continue
        key = name[len("def_"):-len("_radar.svg")]
        svg = re.sub(r">\s+<", "><", dm.read_text(path)).strip()
        svg = re.sub(r"\s{2,}", " ", svg)
        if not svg.startswith("<svg") or "<script" in svg.lower() or "href" in svg.lower():
            continue  # only plain drawings are accepted
        if "xmlns" not in svg[:200]:
            svg = svg.replace("<svg", '<svg xmlns="http://www.w3.org/2000/svg"', 1)
        if key.startswith("nato_"):
            found["nato"][key[len("nato_"):]] = svg
        elif key.startswith("ussr_"):
            found["ussr"][key[len("ussr_"):]] = svg
        else:
            found["wt"][key] = svg
    return found


def unit_icon_type(unit_id: str, spec: dict, role: str = "") -> str:
    """Which map symbol a unit uses, in NATO/USSR terms (sam, tank, truck, ...)."""
    name, role = unit_id.lower(), role.lower()
    radar = str(spec.get("on_radar_as") or "").lower()
    kind = str(spec.get("unit_type") or "").lower()
    if "aircraftcarrier" in name or "carrier" in radar:
        return "aircraft_carrier"
    for word in ("battleship", "missile_cruiser", "heavy_cruiser", "cruiser", "destroyer", "frigate", "corvette"):
        if word in name or word in radar:
            return "missile_cruiser" if word == "missile_cruiser" else word
    if role == "ship" or kind in ("typeship", "typegunboat", "typetorpedogunboat"):
        return "boat"
    by_radar = {"sam": "sam", "radar": "radar_station", "tracked": "tank", "ifv": "ifv", "wheeled": "truck",
                "mlrs": "mlrs", "tbm_launcher": "tbm", "spaa": "spaa", "airdefence": "spaa"}
    if radar in by_radar:
        # A radar-guided SPAA vehicle reports as "sam"; keep SAM for launchers only.
        if radar == "sam" and (role == "spaa" or kind == "typespaa"):
            return "spaa"
        return by_radar[radar]
    by_kind = {"typespaa": "spaa", "typelighttank": "tank", "typemediumtank": "tank", "typeheavytank": "tank",
               "typetankdestroyer": "tank_destroyer"}
    if kind in by_kind:
        return by_kind[kind]
    if "radar" in role or "radar" in name:
        return "radar_station"
    if "launcher" in role or "_sam_" in name:
        return "sam"
    if "truck" in name or "carrier" in name or "wheeled" in name:
        return "truck"
    return "tank"


def gun_range(dm: Datamine, unit_id: str) -> tuple[float, str] | None:
    """A ground unit's anti-aircraft gun range read straight from its game files.

    Used when the reference extraction has no weapon data for a unit. The
    gun's ``attackMaxDist`` (how far the AI opens fire) is preferred, then its
    ``aimMaxDist``. Returns (metres, weapon id).
    """
    base = unit_id.rsplit("/", 1)[-1].lower()
    folder = "aces.vromfs.bin_u/gamedata/units/tankmodels"
    match = next((path for path in dm.ls(f"{folder}/{base}.blkx")), None)
    if match is None:
        return None
    try:
        unit = dm.read_json(match)
    except Exception:
        return None
    weapons = (unit.get("commonWeapons") or {}).get("Weapon") or []
    if isinstance(weapons, dict):
        weapons = [weapons]
    best: tuple[float, str] | None = None
    for weapon in weapons:
        blk = str(weapon.get("blk") or "").replace("\\", "/").lower() if isinstance(weapon, dict) else ""
        if not blk.startswith("gamedata/weapons/"):
            continue
        path = "aces.vromfs.bin_u/" + blk.removesuffix(".blk") + ".blkx"
        try:
            data = dm.read_json(path)
        except Exception:
            continue
        reach = data.get("attackMaxDist") or data.get("aimMaxDist")
        if isinstance(reach, (int, float)) and reach > 0 and (best is None or reach > best[0]):
            best = (float(reach), blk.rsplit("/", 1)[-1].removesuffix(".blk"))
    return best
