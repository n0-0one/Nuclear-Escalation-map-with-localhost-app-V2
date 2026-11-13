"""Build the app's data pack.

Inputs
------
--raw        folder with the mission extractor's output (map_data*.json,
             mission_logic*.json, presets.json, unit_specs.json, ...)
--manifest   the extractor manifest that lists mission variants
--datamine   local clone of the War Thunder datamine (see datamine.py)
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

import extras  # noqa: E402
from datamine import Datamine, REPOSITORY  # noqa: E402

HERE = Path(__file__).resolve().parent
SCHEMA = 1


def load(path: Path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def dump(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")


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
            {"id": item["name"], "team": int(item.get("team", 0)), "aircraft": item.get("aircraft", ""),
             "p": xz(item["world_pos"]), "alt": r1(item["world_pos"][1])}
            for item in logic.get("bomber_spawns", [])
            if item.get("world_pos")
        ],
        "strike_targets": sorted({item["name"] for item in logic.get("mlrs_tbm_targets", []) if item.get("name") in known}),
        "airfield_hp": {name: rule.get("hp") for name, rule in logic.get("airfield_rules", {}).items() if name in known},
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


def pack_unit_art(models: set[str], source: Path | None, out: Path) -> list[str]:
    available = sorted(path.stem for path in out.glob("*.webp")) if out.exists() else []
    if not source or not (source / "images" / "portraits").exists():
        return available
    from PIL import Image

    out.mkdir(parents=True, exist_ok=True)
    for model in sorted(models):
        picture = source / "images" / "portraits" / f"{model}.png"
        target = out / f"{model}.webp"
        if not picture.exists() or target.exists():
            continue
        with Image.open(picture) as image:
            image = image.convert("RGBA")
            image.thumbnail((360, 360), Image.LANCZOS)
            image.save(target, "WEBP", quality=72, method=6)
    return sorted(path.stem for path in out.glob("*.webp"))


# ---------------------------------------------------------------------- main

def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--raw", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--datamine", type=Path, required=True)
    parser.add_argument("--assets", type=Path)
    parser.add_argument("--assets-commit", default="")
    parser.add_argument("--out", type=Path, default=HERE.parent / "data")
    args = parser.parse_args()

    config = load(HERE / "config.json")
    dm = Datamine(args.datamine)
    out: Path = args.out
    (out / "levels").mkdir(parents=True, exist_ok=True)

    manifest = load(args.manifest)
    raw_id_by_mission = {
        Path(item["path"]).name.removesuffix(".blkx"): item["id"] for item in manifest["mission_variants"]
    }

    def raw_file(prefix: str, raw_id: str) -> Path:
        return args.raw / (f"{prefix}.json" if raw_id == "standard" else f"{prefix}_{raw_id}.json")

    presets = load(args.raw / "presets.json")["presets"]
    era_ids = [preset["id"] for preset in presets]
    eras = []
    for index, preset in enumerate(presets):
        low, high = preset["rank_range"]
        br_low = br_from_rank(low)
        if index == 0:
            br_low = max(br_low, config["event_min_br"])
        eras.append({"id": preset["id"], "label": preset["label"], "rank": [low, high],
                     "br": [br_low, br_from_rank(high)]})

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
        logic_any: dict = {}
        for mission in sorted(level_missions, key=lambda item: item["mirror"]):
            raw_id = raw_id_by_mission.get(mission["mission"])
            if raw_id is None or not raw_file("map_data", raw_id).exists():
                problems.append(f"{mission['mission']}: no extracted data yet")
                continue
            logic = load(raw_file("mission_logic", raw_id))
            logic_any = logic_any or logic
            variant = pack_variant(load(raw_file("map_data", raw_id)), logic, era_ids, mission)
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
    logic = None
    for mission in missions:
        raw_id = raw_id_by_mission.get(mission["mission"])
        if raw_id and raw_file("mission_logic", raw_id).exists():
            logic = load(raw_file("mission_logic", raw_id))
            break
    if logic is None:
        raise SystemExit("No mission logic found; nothing to pack.")

    def lower_names(value):
        """Unit ids are case-insensitive in the game files; keep one spelling."""
        if isinstance(value, dict):
            return {key: (item.lower() if key == "name" and isinstance(item, str) else lower_names(item))
                    for key, item in value.items()}
        if isinstance(value, list):
            return [lower_names(item) for item in value]
        return value

    convoys = lower_names(logic.get("convoys", {}))
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

    specs = {key.lower(): value for key, value in load(args.raw / "unit_specs.json").items()}
    names, hud_names, shop_names = extras.unit_tables(dm, unit_ids)
    weapon_names_file = args.raw / "unit_weapon_display_names.json"
    weapon_names = load(weapon_names_file).get("en", {}) if weapon_names_file.exists() else {}
    units = {
        unit_id: pack_unit(specs.get(unit_id, {"unit_class": unit_id}), names.get(unit_id), weapon_names)
        for unit_id in sorted(unit_ids)
    }
    art = pack_unit_art({unit["model"] for unit in units.values() if unit["model"]}, args.assets, out / "units")

    def convoy_era_label(key: str) -> list[str]:
        """Match a convoy template key such as ``1970_early`` to era ids by label."""
        words = key.lower().split("_")
        return [era["id"] for era in eras if all(word in era["label"].lower() for word in words)]

    core = {
        "schema": SCHEMA,
        "game_version": dm.game_version(),
        "eras": eras,
        "units": units,
        "unit_art": art,
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
    hard = [problem for problem in problems if "no extracted data yet" not in problem]

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
        "files": files,
        "sources": sorted(sources, key=lambda item: item["path"]),
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
