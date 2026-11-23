"""Build the app's data pack.

Inputs
------
--datamine   local clone of the War Thunder datamine (see datamine.py). Unit
             positions for every map, side layout and BR bracket are read
             from it by extract.py.
--raw        optional reference extraction that supplies weapon and radar
             ranges and the mission's rule constants (unit_specs.json,
             mission_logic*.json)
--assets     optional folder with level artwork/terrain/roads; when a level's
             files are missing the previously packed ones are kept

Output: ``data/`` - a few compact JSON files plus WebP images, with a
manifest that carries a SHA-256 for every file.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import shutil
import sys
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import extract  # noqa: E402
import extras  # noqa: E402
from datamine import Datamine, REPOSITORY  # noqa: E402

HERE = Path(__file__).resolve().parent
SCHEMA = 1


def load(path: Path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def dump(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True), encoding="utf-8")


def r1(value: float) -> float:
    """Round to 0.1 m and drop a trailing .0 so the JSON stays small."""
    rounded = round(float(value), 1)
    return int(rounded) if rounded == int(rounded) else rounded


def xz(position) -> list:
    return [r1(position[0]), r1(position[2])]


def br_from_rank(rank: int) -> float:
    return round(1.0 + rank / 3.0, 1)


# ---------------------------------------------------------------- categories

def site_category(site: dict) -> str:
    category = site.get("category")
    if category in ("airfield", "strongpoint", "assembly_area", "fuel_factory", "ammo_factory"):
        return category
    name = str(site.get("name", "")).lower()
    group = str(site.get("source_group", "")).lower()
    if group == "ships":
        return "ship"
    if group == "tankmodels":
        if "tactical_missile" in name or "tbm" in name:
            return "tbm"
        if "mlrs" in name:
            return "mlrs"
    if "longrange_radar_place" in name:
        return "radar_pad"
    if "longrange_radar" in name:
        return "ew_radar"
    if "airfield" in name:
        return "airfield"
    if "fuel_storage" in name:
        return "fuel_factory"
    if "ammo_storage" in name:
        return "ammo_factory"
    if re.search(r"t[12]_base_", name):
        return "assembly_area"
    if "long_sam_site" in name:
        return "long_sam"
    if "short_sam_site" in name:
        return "short_sam"
    if "sam_cover" in name:
        return "sam_cover"
    if "spaa_cover" in name:
        return "spaa_cover"
    return "other"


# --------------------------------------------------------------------- sites

def pack_units(by_era: dict, era_ids: list[str], building: bool) -> list[dict]:
    merged: dict[tuple, dict] = {}
    for bit, era in enumerate(era_ids):
        for unit in by_era.get(era, []) or []:
            position = unit.get("world_pos")
            if not position:
                continue
            life = (unit.get("lifecycle") or {}).get("kind", "none")
            unit_id = str(unit.get("name") or "").lower()
            key = (unit_id, unit.get("role"), round(position[0], 1), round(position[2], 1), life)
            entry = merged.get(key)
            if entry is None:
                entry = {"u": unit_id, "p": xz(position), "e": 0}
                if not building:
                    entry["r"] = unit.get("role") or "vehicle"
                    if life != "none":
                        entry["l"] = life
                    if unit.get("threat_range"):
                        entry["tr"] = r1(unit["threat_range"])
                    if unit.get("unplaced"):
                        entry["x"] = 1
                merged[key] = entry
            entry["e"] |= 1 << bit
    return list(merged.values())


def pack_site(site: dict, era_ids: list[str], capture: dict) -> dict:
    category = site_category(site)
    packed = {
        "id": site["name"],
        "cls": site.get("unit_class", ""),
        "team": int(site.get("team", 0)),
        "cat": category,
        "p": xz(site["world_pos"]),
        "alt": r1(site["world_pos"][1]),
        "units": pack_units(site.get("units_by_era", {}), era_ids, building=False),
    }
    buildings = pack_units(site.get("buildings_by_era", {}), era_ids, building=True)
    if buildings:
        packed["bld"] = buildings
    if site.get("runways"):
        packed["rw"] = [
            [*xz(runway["start"]), *xz(runway["end"]), r1(runway.get("width", 80))]
            for runway in site["runways"]
        ]
    route = site.get("route")
    if isinstance(route, dict) and route.get("points"):
        points: list = []
        for point in route["points"]:
            points.extend(xz(point["world_pos"]))
        packed["route"] = {
            "pts": points,
            "closed": bool(route.get("closed")),
            "speed_kph": r1(route.get("speed") or 0),
        }
    if site.get("runtime_relocated"):
        packed["moves"] = site.get("runtime_position_kind", "relocated")
    if site["name"] in capture:
        packed["cap"] = r1(capture[site["name"]])
    return packed


def pack_variant(raw_sites: list, logic: dict, era_ids: list[str], mission: dict) -> dict:
    capture = {
        site["name"]: site["capture_radius"]
        for site in logic.get("sites", [])
        if site.get("capture_radius")
    }
    sites = [pack_site(site, era_ids, capture) for site in raw_sites if site.get("world_pos")]
    known = {site["id"] for site in sites}

    def links(items: list, source: str, target: str) -> list:
        return [
            [item[source], item[target], int(item.get("rank_by_distance", 0))]
            for item in items
            if item.get(source) in known and item.get(target) in known
        ]

    def area(value: dict | None):
        if not isinstance(value, dict) or not value.get("center"):
            return None
        return [*xz(value["center"]), r1(value.get("radius", 0))]

    mobile = []
    for unit in logic.get("mobile_fire_units", []):
        mobile.append(
            {
                "id": unit["name"],
                "team": int(unit.get("team", 0)),
                "kind": unit.get("unit_type", ""),
                "spawn": area(unit.get("spawn_area")),
                "move": [a for a in (area(item) for item in unit.get("move_areas", [])) if a],
            }
        )
    return {
        "id": "mirror" if mission["mirror"] else "standard",
        "mission": mission["mission"],
        "mirror": mission["mirror"],
        "detect_loss_s": mission["settings"].get("timeToLoseInstantDetection", 10.0),
        "respawn_delay_s": mission["settings"].get("afterRespawnDelaySec"),
        "score_limit": mission["settings"].get("scoreLimit"),
        "time_limit_min": mission["settings"].get("timeLimit"),
        "escalation": [
            mission["settings"][key]
            for key in ("nuclearEscalationStage1", "nuclearEscalationStage2", "nuclearEscalationStage3")
            if key in mission["settings"]
        ],
        "sites": sites,
        "supply_links": links(logic.get("likely_convoy_links", []), "factory", "assembly"),
        "armor_links": links(logic.get("likely_armored_links", []), "assembly", "target"),
        "mobile": mobile,
        "bombers": [
            {"id": item["name"], "team": int(item.get("team", 0)),
             "p": xz(item["world_pos"]), "alt": r1(item["world_pos"][1])}
            for item in logic.get("bomber_spawns", [])
            if item.get("world_pos")
        ],
        "strike_targets": sorted({item["name"] for item in logic.get("mlrs_tbm_targets", []) if item.get("name") in known}),
    }


# --------------------------------------------------------------------- roads

def simplify(points: list, tolerance: float) -> list:
    """Douglas-Peucker in the horizontal plane; keeps each point's height."""
    if len(points) < 3:
        return points
    keep = [False] * len(points)
    keep[0] = keep[-1] = True
    stack = [(0, len(points) - 1)]
    while stack:
        first, last = stack.pop()
        ax, az = points[first][0], points[first][2]
        bx, bz = points[last][0], points[last][2]
        dx, dz = bx - ax, bz - az
        length = math.hypot(dx, dz) or 1e-9
        worst, index = 0.0, -1
        for i in range(first + 1, last):
            px, pz = points[i][0] - ax, points[i][2] - az
            along = max(0.0, min(length, (px * dx + pz * dz) / length))
            distance = math.hypot(px - dx / length * along, pz - dz / length * along)
            if distance > worst:
                worst, index = distance, i
        if worst > tolerance and index > 0:
            keep[index] = True
            stack.append((first, index))
            stack.append((index, last))
    return [point for point, flag in zip(points, keep) if flag]


def pack_roads(roads: list) -> list:
    packed = []
    for road in roads:
        points = simplify(road.get("points") or [], 2.0)
        if len(points) < 2:
            continue
        flat: list[int] = []
        for point in points:
            flat.extend((round(point[0]), round(point[1]), round(point[2])))
        packed.append({"s": road.get("surface") or "unclassified", "p": flat})
    return packed


def pack_navmesh(mesh: dict) -> dict | None:
    vertices, triangles = mesh.get("vertices") or [], mesh.get("triangles") or []
    if not vertices or not triangles:
        return None
    flat_vertices: list[int] = []
    for vertex in vertices:
        flat_vertices.extend((round(vertex[0]), round(vertex[2])))
    flat_triangles: list[int] = []
    for triangle in triangles:
        flat_triangles.extend(int(index) for index in triangle[:3])
    return {"v": flat_vertices, "t": flat_triangles}


# --------------------------------------------------------------------- units

def pack_unit(spec: dict, names: list | None, weapon_names: dict) -> dict:
    sensors = spec.get("sensors") or {}

    def reach(kind: str):
        sensor = sensors.get(kind) or {}
        value = sensor.get("maximum") or sensor.get("nominal") or sensor.get("designation_range")
        return r1(value) if value else None

    weapons = []
    for weapon in spec.get("weapons") or []:
        entry = {"n": weapon_names.get(weapon.get("name"), weapon.get("name", ""))}
        for source, target in (("ammo", "ammo"), ("minimum", "min"), ("effective_maximum", "max"),
                               ("guidance", "g"), ("reload_seconds", "rl")):
            if weapon.get(source) not in (None, 0, ""):
                entry[target] = weapon[source]
        weapons.append(entry)

    unit = {
        "n": (names or [None])[0] or spec.get("model") or spec.get("unit_class"),
        "icon": spec.get("on_radar_as") or "",
        "type": (spec.get("unit_type") or "").removeprefix("type"),
        "model": spec.get("model") or "",
    }
    if names and names[1] and names[1] != unit["n"]:
        unit["s"] = names[1]
    for key, value in (
        ("det", reach("detection")),
        ("trk", reach("tracking")),
        ("eng", spec.get("engagement_range")),
        ("ai_eng", spec.get("ai_engagement_range")),
        ("gun", spec.get("ai_gun_range")),
        ("kph", spec.get("maximum_speed_kph")),
    ):
        if value:
            unit[key] = r1(value)
    if weapons:
        unit["w"] = weapons
    return unit


# -------------------------------------------------------------------- images

def pack_images(level: str, assets: dict, source: Path | None, out: Path, config: dict) -> dict:
    """Convert level artwork. Returns the fields to merge into the level file."""
    result: dict = {}
    image_out = out / f"{level}.webp"
    height_out = out / f"{level}.height.webp"
    if source and assets.get("image") and (source / assets["image"]).exists():
        from PIL import Image

        Image.MAX_IMAGE_PIXELS = None
        size = config["image"]["size"]
        with Image.open(source / assets["image"]) as image:
            image = image.convert("RGB")
            if max(image.size) > size:
                image = image.resize((size, size), Image.LANCZOS)
            image.save(image_out, "WEBP", quality=config["image"]["quality"], method=6)
    if image_out.exists():
        result["image"] = image_out.name

    if source and assets.get("height") and assets.get("height_meta"):
        height_src, meta_src = source / assets["height"], source / assets["height_meta"]
        if height_src.exists() and meta_src.exists():
            shutil.copyfile(height_src, height_out)
            meta = load(meta_src)
            encoding = meta["height_encoding"]
            dump(out / f"{level}.height.json", {
                "w": encoding["width"], "h": encoding["height"],
                "min_m": encoding["min_m"], "step_m": encoding["step_m"],
                "water": meta.get("water_level_m", 0.0),
            })
    meta_out = out / f"{level}.height.json"
    if height_out.exists() and meta_out.exists():
        result["terrain"] = {"image": height_out.name, **load(meta_out)}
    return result


GENERIC_ART = {
    # class -> filled polygons on a 360 x 200 canvas (drawn for this project)
    "truck": [[(40, 150), (40, 80), (190, 80), (190, 150)], [(198, 150), (198, 100), (250, 100), (285, 125), (300, 125), (300, 150)]],
    "tank": [[(40, 150), (60, 118), (300, 118), (320, 150)], [(110, 118), (125, 85), (215, 85), (232, 118)], [(215, 92), (335, 92), (335, 102), (215, 102)]],
    "launcher": [[(40, 150), (40, 122), (300, 122), (300, 150)], [(70, 118), (60, 104), (270, 52), (280, 68)], [(250, 122), (250, 96), (296, 96), (296, 122)]],
    "sam": [[(60, 150), (60, 128), (300, 128), (300, 150)], [(120, 124), (112, 108), (262, 46), (272, 62)], [(96, 128), (150, 96), (164, 106), (130, 128)]],
    "radar": [[(60, 150), (60, 122), (300, 122), (300, 150)], [(172, 122), (172, 70), (188, 70), (188, 122)], [(100, 70), (260, 70), (236, 34), (124, 34)]],
    "ship": [[(20, 128), (340, 128), (300, 164), (56, 164)], [(110, 128), (110, 98), (240, 98), (240, 128)], [(160, 98), (160, 56), (176, 56), (176, 98)], [(60, 128), (60, 112), (100, 112), (100, 128)]],
    "carrier": [[(14, 124), (346, 124), (318, 164), (50, 164)], [(214, 124), (214, 82), (262, 82), (262, 124)], [(232, 82), (232, 54), (244, 54), (244, 82)]],
}
GENERIC_WHEELS = {"truck": [70, 110, 230, 275], "launcher": [70, 120, 220, 270], "sam": [90, 150, 210, 270], "radar": [90, 150, 210, 270]}


def art_class(spec: dict) -> str:
    """Which stand-in drawing suits a unit that has no artwork."""
    icon, name = str(spec.get("on_radar_as") or "").lower(), str(spec.get("unit_class") or "").lower()
    if "carrier" in icon or "aircraftcarrier" in name:
        return "carrier"
    if any(word in icon for word in ("ship", "destroyer", "frigate", "cruiser", "boat", "corvette")) or spec.get("unit_type") in ("typeDestroyer", "typeFrigate"):
        return "ship"
    if "wheeled" in icon or "carrier" in name or "truck" in name:
        return "truck"
    if "tbm" in icon or "mlrs" in icon:
        return "launcher"
    if "radar" in icon:
        return "radar"
    if icon in ("sam", "airdefence", "spaa"):
        return "sam"
    return "tank"


def pack_unit_art(units: dict[str, dict], specs: dict, source: Path | None, dm: Datamine, out: Path) -> None:
    """Gives every unit a picture: ``units[id]["img"]`` names a file in ``units/``.

    In order: the reference project's vehicle art, the game's own small
    vehicle icon from the datamine, and last a plain stand-in drawing of the
    unit's class.
    """
    from PIL import Image, ImageDraw

    out.mkdir(parents=True, exist_ok=True)
    portraits = {}
    folder = source / "images" / "portraits" if source else None
    if folder and folder.exists():
        portraits = {path.stem.lower(): path for path in folder.glob("*.png")}
    icons = {path.rsplit("/", 1)[-1][:-4].lower(): path for path in dm.ls("atlases.vromfs.bin_u/units") if path.endswith(".png")}
    import io

    def save(image, target: Path) -> None:
        image = image.convert("RGBA")
        image.thumbnail((360, 360), Image.LANCZOS)
        image.save(target, "WEBP", quality=72, method=6)

    for unit_id, unit in units.items():
        base = unit_id.rsplit("/", 1)[-1].lower()
        model = unit["model"].lower()
        # the small tech-tree picture the game shows for the vehicle
        slot_key = next((key for key in (base, base.removesuffix("_ai"), base.removesuffix("_ec"), model) if key in icons), None)
        if slot_key:
            slot_file = out / f"slot_{slot_key}.webp"
            if not slot_file.exists():
                with Image.open(io.BytesIO(dm.read(icons[slot_key]))) as image:
                    image.convert("RGBA").save(slot_file, "WEBP", quality=85, method=6)
            unit["slot"] = f"slot_{slot_key}"
        stem = model or base
        target = out / f"{stem}.webp"
        if not target.exists():
            portrait = portraits.get(model) or portraits.get(base)
            icon = next((icons[key] for key in (base, base.removesuffix("_ai"), model) if key in icons), None)
            if portrait:
                with Image.open(portrait) as image:
                    save(image, target)
            elif icon:
                with Image.open(io.BytesIO(dm.read(icon))) as image:
                    save(image, target)
        if target.exists():
            unit["img"] = stem
            continue
        kind = art_class(specs.get(unit_id, {"unit_class": unit_id}))
        generic = out / f"_{kind}.webp"
        if not generic.exists():
            scale = 2
            image = Image.new("RGBA", (360 * scale, 200 * scale), (0, 0, 0, 0))
            draw = ImageDraw.Draw(image)
            fill = (176, 184, 192, 255)
            for polygon in GENERIC_ART[kind]:
                draw.polygon([(x * scale, y * scale) for x, y in polygon], fill=fill)
            for x in GENERIC_WHEELS.get(kind, []):
                draw.ellipse([(x - 15) * scale, 138 * scale, (x + 15) * scale, 168 * scale], fill=fill)
            if kind == "tank":
                draw.rounded_rectangle([40 * scale, 146 * scale, 320 * scale, 170 * scale], radius=12 * scale, fill=fill)
            image.resize((360, 200), Image.LANCZOS).save(generic, "WEBP", quality=80, method=6)
        unit["img"] = f"_{kind}"
        unit["img_generic"] = True


# ---------------------------------------------------------------------- main

def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--raw", "--reference", type=Path, dest="raw",
                        help="reference extraction with unit ranges and mission rules")
    parser.add_argument("--datamine", type=Path, required=True)
    parser.add_argument("--assets", type=Path)
    parser.add_argument("--assets-commit", default="")
    parser.add_argument("--out", type=Path, default=HERE.parent / "data")
    parser.add_argument("--pending", default="[]", help="JSON list of event files that changed after extraction")
    args = parser.parse_args()

    config = load(HERE / "config.json")
    dm = Datamine(args.datamine)
    out: Path = args.out
    (out / "levels").mkdir(parents=True, exist_ok=True)

    # The reference extraction supplies what this project does not derive
    # itself yet: weapon and radar ranges and the mission's rule constants.
    reference = args.raw if args.raw and (args.raw / "unit_specs.json").exists() else None
    reference_logic = {}
    if reference:
        for candidate in sorted(reference.glob("mission_logic*.json")):
            reference_logic = load(candidate)
            break
    capture_radii = {**config["capture_radius_fallback"], **reference_logic.get("rules", {}).get("capture", {}).get("radii", {})}

    source = extract.Source(dm)
    eras: list[dict] = []
    era_ids: list[str] = []

    missions = extras.discover_missions(dm, config)
    official_names = extras.level_names(dm)
    sources = [{"path": m["path"], "sha": m["sha"]} for m in missions]
    unit_ids: set[str] = set()
    levels_index = []
    by_level: dict[str, list[dict]] = {}
    for mission in missions:
        by_level.setdefault(mission["level"], []).append(mission)

    problems: list[str] = []
    for level, level_missions in sorted(by_level.items()):
        meta = extras.level_meta(dm, level)
        if meta is None:
            problems.append(f"{level}: level definition missing from the datamine")
            continue
        sources.append({"path": meta["path"], "sha": meta["sha"]})
        level_id = level.removeprefix("air_")
        variants = []
        for mission in sorted(level_missions, key=lambda item: item["mirror"]):
            try:
                raw_sites, mission_eras, document = extract.extract_mission(source, mission["path"])
            except Exception as error:  # one unreadable mission must not sink the others
                problems.append(f"{mission['mission']}: could not be read ({error})")
                continue
            if not eras:
                for index, era in enumerate(mission_eras):
                    low, high = era["rank_range"]
                    br_low = max(br_from_rank(low), config["event_min_br"]) if index == 0 else br_from_rank(low)
                    eras.append({"id": era["id"], "label": era["label"], "rank": [low, high], "br": [br_low, br_from_rank(high)]})
                era_ids = [era["id"] for era in eras]
            elif [era["id"] for era in mission_eras] != era_ids:
                problems.append(f"{mission['mission']}: uses different BR brackets than the other missions")
                continue
            logic = extract.mission_logic(document, raw_sites, capture_radii)
            variant = pack_variant(raw_sites, logic, era_ids, mission)
            for site in variant["sites"]:
                unit_ids.update(unit["u"] for unit in site["units"])
            variants.append(variant)
        if not variants:
            continue

        assets = config["level_assets"].get(level, {})
        document = {
            "schema": SCHEMA,
            "id": level_id,
            "level": level,
            "name": config["level_names"].get(level) or official_names.get(level) or level_id.replace("_", " ").title(),
            "bounds": meta["bounds"],
            "water": meta["water"],
            "grid_step": meta["grid_step"],
            "variants": variants,
        }
        document.update(pack_images(level_id, assets, args.assets, out / "levels", config))
        roads_out = out / "levels" / f"{level_id}.roads.json"
        if args.assets and assets.get("roads") and (args.assets / assets["roads"]).exists():
            roads = {"roads": pack_roads(load(args.assets / assets["roads"]))}
            if assets.get("navmesh") and (args.assets / assets["navmesh"]).exists():
                navmesh = pack_navmesh(load(args.assets / assets["navmesh"]))
                if navmesh:
                    roads["navmesh"] = navmesh
            dump(roads_out, roads)
        if roads_out.exists():
            document["roads"] = roads_out.name
        dump(out / "levels" / f"{level_id}.json", document)
        levels_index.append({
            "id": level_id, "level": level, "name": document["name"], "bounds": meta["bounds"],
            "file": f"levels/{level_id}.json",
            "has_image": "image" in document, "has_terrain": "terrain" in document, "has_roads": "roads" in document,
            "variants": [variant["id"] for variant in variants],
        })

    # ------------------------------------------------------------- core file
    logic = reference_logic
    if not levels_index:
        raise SystemExit("No mission could be read; nothing to pack.")

    def lower_names(value):
        """Unit ids are case-insensitive in the game files; keep one spelling."""
        if isinstance(value, dict):
            return {key: (item.lower() if key == "name" and isinstance(item, str) else lower_names(item))
                    for key, item in value.items()}
        if isinstance(value, list):
            return [lower_names(item) for item in value]
        return value

    # Convoy makeups straight from the game's factory templates; the
    # reference extraction is only a fallback if those cannot be read.
    convoys = lower_names(extras.convoy_compositions(dm) or logic.get("convoys", {}))
    for group in convoys.values():
        for era in group.values():
            if not isinstance(era, dict):
                continue
            for kind in era.values():
                if not isinstance(kind, dict):
                    continue
                for side in kind.values():
                    if isinstance(side, list):
                        unit_ids.update(item["name"] for item in side if item.get("name"))
                    elif isinstance(side, dict):
                        for entries in side.values():
                            if isinstance(entries, list):
                                unit_ids.update(item["name"] for item in entries if item.get("name"))

    specs = {key.lower(): value for key, value in load(reference / "unit_specs.json").items()} if reference else {}
    names, hud_names, shop_names = extras.unit_tables(dm, unit_ids)
    weapon_names_file = reference / "unit_weapon_display_names.json" if reference else None
    weapon_names = load(weapon_names_file).get("en", {}) if weapon_names_file and weapon_names_file.exists() else {}
    # Units the reference has no weapon data for (new, or changed in a recent
    # patch) are read straight from the game's files.
    for unit_id in sorted(unit_ids):
        spec = specs.get(unit_id)
        has_reach = spec and (spec.get("engagement_range") or spec.get("ai_engagement_range") or spec.get("ai_gun_range"))
        if has_reach or (spec and spec.get("sensors")):
            continue
        found = extras.unit_from_game(dm, unit_id)
        if found and (found.get("engagement_range") or found.get("ai_gun_range") or not spec):
            specs[unit_id] = {**(spec or {}), **{key: value for key, value in found.items() if value}}
    units = {
        unit_id: pack_unit(specs.get(unit_id, {"unit_class": unit_id}), names.get(unit_id), weapon_names)
        for unit_id in sorted(unit_ids)
    }
    pack_unit_art(units, specs, args.assets, dm, out / "units")
    roles: dict[str, str] = {}
    for level in levels_index:
        for variant in load(out / level["file"])["variants"]:
            for site in variant["sites"]:
                for placement in site["units"]:
                    roles.setdefault(placement["u"], placement.get("r", ""))
    for unit_id, unit in units.items():
        unit["it"] = extras.unit_icon_type(unit_id, specs.get(unit_id, {"unit_class": unit_id}), roles.get(unit_id, ""))
    dump(out / "icons.json", extras.map_icons(dm))

    def convoy_era_label(key: str) -> list[str]:
        """Match a convoy template key such as ``1970_early`` to era ids by label."""
        words = key.lower().split("_")
        return [era["id"] for era in eras if all(word in era["label"].lower() for word in words)]

    core = {
        "schema": SCHEMA,
        "game_version": dm.game_version(),
        "eras": eras,
        "units": units,
        # Read by app version 2.0; newer versions use each unit's "img".
        "unit_art": sorted({unit["img"] for unit in units.values() if not unit.get("img_generic")}),
        "convoys": {
            group: [{"eras": convoy_era_label(key), "key": key, **value} for key, value in items.items()]
            for group, items in convoys.items()
        },
        "rules": logic.get("rules", {}),
        "respawn": logic.get("respawn_rules", []),
        "template_restore": logic.get("template_restore", {}),
        "no_fixed_respawn": logic.get("no_fixed_respawn", []),
        "hud": {"verbs": extras.kill_verbs(dm), "names": hud_names},
        "vehicles": extras.vehicles(dm, shop_names),
        "nukes": extras.nuclear_weapons(dm, config["nuclear_weapon_name_hints"]),
        "levels": levels_index,
    }
    dump(out / "core.json", core)

    # ------------------------------------------------------------ validation
    if not levels_index:
        problems.append("no level could be packed")
    for level in levels_index:
        document = load(out / level["file"])
        for variant in document["variants"]:
            teams = {site["team"] for site in variant["sites"]}
            airfields = [site for site in variant["sites"] if site["cat"] == "airfield"]
            if teams != {1, 2}:
                problems.append(f"{level['id']}/{variant['id']}: expected two teams, found {sorted(teams)}")
            if len(airfields) < 2:
                problems.append(f"{level['id']}/{variant['id']}: fewer than two airfields")
            low_x, low_z, high_x, high_z = document["bounds"]
            for site in variant["sites"]:
                x, z = site["p"]
                if not (low_x <= x <= high_x and low_z <= z <= high_z):
                    problems.append(f"{level['id']}/{variant['id']}: {site['id']} is outside the map")
            for bit, era in enumerate(era_ids):
                count = sum(1 for site in variant["sites"] for unit in site["units"] if unit["e"] >> bit & 1)
                if count < 50:
                    problems.append(f"{level['id']}/{variant['id']}: only {count} units in {era}")
    missing_specs = sorted(unit_id for unit_id in unit_ids if unit_id not in specs)
    hard = list(problems)

    files = {}
    for path in sorted(out.rglob("*")):
        if path.is_file() and path.name != "manifest.json":
            files[path.relative_to(out).as_posix()] = {"sha256": sha256(path), "bytes": path.stat().st_size}
    previous = load(out / "manifest.json") if (out / "manifest.json").exists() else {}
    document = {
        "schema": SCHEMA,
        "game_version": core["game_version"],
        "datamine": {"repository": REPOSITORY, "commit": dm.commit},
        "terrain_source": {
            "repository": config["terrain_source"]["repository"],
            "commit": args.assets_commit or previous.get("terrain_source", {}).get("commit", ""),
        },
        "pending_game_changes": json.loads(args.pending),
        "files": files,
        "sources": sorted({item["path"]: item for item in [*sources, *({"path": path, "sha": sha} for path, sha in source.used.items())]}.values(),
                          key=lambda item: item["path"]),
    }
    # The timestamp only changes when the content does, so an unchanged
    # datamine never produces a new pack revision.
    comparable = {key: value for key, value in document.items()}
    old = {key: value for key, value in previous.items() if key != "generated"}
    document["generated"] = previous.get("generated") if comparable == old and previous.get("generated") else \
        datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    (out / "manifest.json").write_text(json.dumps(document, indent=1) + "\n", encoding="utf-8")

    total = sum(item["bytes"] for item in files.values())
    print(f"Packed {len(levels_index)} levels, {len(units)} units, {len(core['vehicles'])} vehicles, "
          f"{len(hud_names)} kill-feed names, {total / 1e6:.1f} MB in {len(files)} files")
    for problem in problems:
        print("WARNING:" if problem not in hard else "ERROR:", problem)
    if missing_specs:
        print("WARNING: no specs for", ", ".join(missing_specs))
    return 1 if hard else 0


if __name__ == "__main__":
    raise SystemExit(main())
