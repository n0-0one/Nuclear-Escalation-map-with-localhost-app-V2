"""Reads Nuclear Escalation missions straight from the datamine.

For every mission this works out, per BR bracket, which units stand where:

* a mission places *sites* (object groups such as a SAM site or an airfield);
* each site names game templates, some of them only for a range of battle
  ranks - that is what makes the brackets differ;
* a template lists the vehicles to spawn and the kind of slot each goes in;
* the site's object-group file gives the slot positions relative to the site.

Ships, long-range radars, rocket artillery and missile launchers are placed
directly by the mission, partly through rank-dependent imported files.

The result has one entry per site with its units for every bracket.
"""

from __future__ import annotations

import subprocess
from collections import Counter

from datamine import Datamine

TEMPLATE_DIR = "aces.vromfs.bin_u/templates/nuclear_escalation"
OBJECT_GROUP_DIR = "aces.vromfs.bin_u/gamedata/objectgroups"
MISSION_ROOT = "mis.vromfs.bin_u/"
STORAGE_SLOTS = {"fuel_storage": "assembly_area__fuelStorageUnitName", "ammo_storage": "assembly_area__ammoStorageUnitName"}
CATEGORY_COMPONENTS = (
    ("assembly_area", "assembly_area"),
    ("fuel_factory", "fuel_factory"),
    ("ammo_factory", "ammo_factory"),
    ("strongpoint", "strongpoint"),
    ("nuclear_escalation_airfield", "airfield"),
)


def as_list(value) -> list:
    if value is None:
        return []
    return value if isinstance(value, list) else [value]


def is_tm(value) -> bool:
    return (
        isinstance(value, list) and len(value) >= 4
        and all(isinstance(row, list) and len(row) >= 3 and all(isinstance(n, (int, float)) for n in row[:3]) for row in value[:4])
    )


def to_world(tm, local) -> list[float]:
    """Applies a site's transform to a position given relative to the site."""
    x, y, z = local
    return [tm[3][i] + x * tm[0][i] + y * tm[1][i] + z * tm[2][i] for i in range(3)]


def walk(value):
    """Every (key, value) pair anywhere inside nested dicts and lists."""
    if isinstance(value, dict):
        for key, child in value.items():
            yield key, child
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def in_rank(config, rank: int) -> bool:
    bounds = config.get("rankRange") if isinstance(config, dict) else None
    if not (isinstance(bounds, list) and len(bounds) >= 2):
        return True
    return bounds[0] <= rank <= bounds[1]


class Source:
    """Datamine documents, loaded on demand and cached."""

    def __init__(self, dm: Datamine):
        self.dm = dm
        self.cache: dict[str, dict | None] = {}
        self.used: dict[str, str] = {}
        self.templates: dict[str, dict] = {}
        for path in sorted(dm.ls(TEMPLATE_DIR)):
            for name, definition in (self.load(path) or {}).items():
                if isinstance(definition, dict):
                    self.templates[name] = definition
        self.layouts: dict[str, dict] = {}

    def load(self, path: str) -> dict | None:
        if path not in self.cache:
            try:
                document = self.dm.read_json(path)
                self.used[path] = self.dm.blob_sha(path)
            except (subprocess.CalledProcessError, ValueError):
                document = None
            self.cache[path] = document if isinstance(document, dict) else None
        return self.cache[path]

    def import_path(self, record) -> str | None:
        file = record.get("file") if isinstance(record, dict) else None
        if not isinstance(file, str) or not file.strip():
            return None
        file = file.strip().replace("\\", "/").lstrip("/")
        return MISSION_ROOT + (file[:-4] + ".blkx" if file.endswith(".blk") else file)

    def imports(self, document: dict, rank: int | None) -> list[tuple[dict, bool]]:
        """Documents reachable through imports: (document, contributes units)."""
        found: list[tuple[dict, bool]] = []
        seen = {id(document)}
        queue: list[tuple[dict, bool]] = [(document, False)]
        while queue:
            current, with_units = queue.pop(0)
            records = as_list((current.get("imports") or {}).get("import_record")) if isinstance(current.get("imports"), dict) else []
            for record in records:
                if not isinstance(record, dict) or (rank is not None and not in_rank(record, rank)):
                    continue
                path = self.import_path(record)
                child = self.load(path) if path else None
                if child is None or id(child) in seen:
                    continue
                seen.add(id(child))
                # Event files carry real units even without the importUnits flag.
                units = with_units or bool(record.get("importUnits")) or "nuclear_escalation" in path.rsplit("/", 1)[-1]
                found.append((child, units))
                queue.append((child, units))
        return found

    # ------------------------------------------------------------ templates
    def closure(self, names) -> list[tuple[str, dict]]:
        """The named templates with everything they inherit or refer to."""
        ordered: list[tuple[str, dict]] = []
        visited: set[str] = set()

        def visit(name):
            if not isinstance(name, str):
                return
            for part in name.split("+"):
                if part in visited or part not in self.templates:
                    continue
                visited.add(part)
                definition = self.templates[part]
                for base in as_list(definition.get("_use")):
                    visit(base)
                ordered.append((part, definition))
                for key, value in walk(definition):
                    if isinstance(value, str) and key.endswith("Templ"):
                        visit(value)

        for name in names:
            visit(name)
        return ordered

    def components(self, names) -> set[str]:
        """Every template name the given ones build on, including base
        templates that are defined elsewhere in the game (such as the generic
        ``restore_unit_by_timer``)."""
        found: set[str] = set()

        def visit(name):
            if not isinstance(name, str):
                return
            for part in name.split("+"):
                if part in found:
                    continue
                found.add(part)
                for base in as_list(self.templates.get(part, {}).get("_use")):
                    visit(base)

        for name in names:
            visit(name)
        return found

    def category(self, template_names) -> str | None:
        closure = self.closure(template_names)
        components = {name for name, _ in closure}
        for _, definition in closure:
            components.update(key.split(":", 1)[0] for key, _ in walk(definition) if key.endswith(":tag"))
        return next((category for component, category in CATEGORY_COMPONENTS if component in components), None)

    def spawn_settings(self, template_names, team) -> list[dict]:
        settings, seen = [], set()
        for preset, definition in self.closure(template_names):
            for key, value in walk(definition):
                if not (key.startswith("units_spawn_on_init__unitSettings") or key == "sam_site__unitSettings:array"):
                    continue
                if ("Team1" in key and team != 1) or ("Team2" in key and team != 2) or not isinstance(value, dict):
                    continue
                for unit in as_list(value.get("unit:object")):
                    if not isinstance(unit, dict) or not unit.get("name"):
                        continue
                    signature = repr(sorted(unit.items(), key=lambda item: item[0]))
                    if signature not in seen:
                        seen.add(signature)
                        settings.append({**unit, "_preset": preset})
        return settings

    def values(self, template_names, keys: set[str]) -> dict:
        found = {}
        for _, definition in self.closure(template_names):
            for key, value in walk(definition):
                if key in keys:
                    found[key] = value
        return found

    # -------------------------------------------------------------- layouts
    def layout(self, object_class: str) -> dict:
        if object_class not in self.layouts:
            document = self.load(f"{OBJECT_GROUP_DIR}/{object_class}.blkx") or {}
            slots, seen = [], set()

            def add(kind, tm):
                if not is_tm(tm):
                    return
                key = tuple(round(float(v), 5) for v in tm[3][:3])
                if key not in seen:
                    seen.add(key)
                    slots.append({"kind": str(kind).lower(), "pos": tm[3][:3]})

            spawns = document.get("ecsUnitSpawnPositions")
            for kind, value in (spawns.items() if isinstance(spawns, dict) else []):
                for tm in ([value] if is_tm(value) else as_list(value)):
                    add(kind, tm)
            for node in as_list(document.get("node")):
                if isinstance(node, dict) and node.get("type"):
                    add(node["type"], node.get("tm"))
            airfield = document.get("airfield")
            runway = None
            if isinstance(airfield, dict) and "start" in airfield and "end" in airfield:
                runway = {"start": airfield["start"], "end": airfield["end"], "width": airfield.get("width", 80.0)}
            self.layouts[object_class] = {"slots": slots, "runway": runway}
        return self.layouts[object_class]


def site_templates(site: dict, rank: int) -> tuple[list[str], list[dict]]:
    names, configs = [], []
    additional = site.get("additionalEcsTemplates")
    for name, value in (additional.items() if isinstance(additional, dict) else []):
        for config in as_list(value):
            if in_rank(config, rank):
                names.append(name)
                configs.append(config if isinstance(config, dict) else {})
    return names, configs


def discover_eras(mission: dict) -> list[dict]:
    """Non-overlapping rank ranges, cut wherever any site's templates change."""
    groups = [g for g in as_list((mission.get("units") or {}).get("objectGroups")) if isinstance(g, dict)]
    ranges = []
    for site in groups:
        additional = site.get("additionalEcsTemplates")
        for value in (additional.values() if isinstance(additional, dict) else []):
            for config in as_list(value):
                bounds = config.get("rankRange") if isinstance(config, dict) else None
                if isinstance(bounds, list) and len(bounds) >= 2 and all(isinstance(b, int) for b in bounds[:2]):
                    ranges.append((bounds[0], bounds[1]))
    if not ranges:
        return [{"id": "rank-0-50", "label": "All ranks", "rank": 0, "rank_range": [0, 50]}]
    cuts = sorted({point for low, high in ranges for point in (low, high + 1)})
    eras = []
    for low, nxt in zip(cuts, cuts[1:]):
        if not any(a <= low <= b for a, b in ranges):
            continue
        text = " ".join(name for site in groups for name in site_templates(site, low)[0]).lower()
        year = next((y for y in ("1970", "1980", "2018") if y in text), None)
        phase = " Early" if "early" in text else " Late" if "late" in text else ""
        eras.append({"id": f"rank-{low}-{nxt - 1}", "label": f"{year}{phase}" if year else f"Rank {low}-{nxt - 1}",
                     "rank": low, "rank_range": [low, nxt - 1]})
    totals, index = Counter(era["label"] for era in eras), Counter()
    for era in eras:
        if totals[era["label"]] > 1:
            index[era["label"]] += 1
            era["label"] = f"{era['label']} {index[era['label']]}"
    return eras


def rank_test(condition: dict, rank: int) -> bool:
    value = condition.get("value")
    if not isinstance(value, (int, float)):
        return True
    return {
        "more": rank > value, "less": rank < value, "equal": rank == value, "notequal": rank != value,
        "moreequal": rank >= value, "lessequal": rank <= value,
    }.get(str(condition.get("comparasion_func", "")).lower(), True)


def spawn_rank_rules(documents: list[dict]) -> dict[str, list[dict]]:
    """Units that a trigger only spawns for certain battle ranks."""
    rules: dict[str, list[dict]] = {}
    for document in documents:
        triggers = document.get("triggers")
        for trigger in (triggers.values() if isinstance(triggers, dict) else []):
            if not isinstance(trigger, dict):
                continue
            conditions = trigger.get("conditions")
            checks = [c for c in as_list(conditions.get("varCompareInt") if isinstance(conditions, dict) else None)
                      if isinstance(c, dict) and c.get("var_value") == "mission_rank"]
            actions = trigger.get("actions")
            if not checks or not isinstance(actions, dict):
                continue
            for kind in ("unitRespawn", "unitSpawnOnObjectGroup"):
                for action in as_list(actions.get(kind)):
                    if isinstance(action, dict) and isinstance(action.get("object"), str):
                        rules.setdefault(action["object"], []).extend(checks)
    return rules


def spawn_area_name(unit_name: str) -> str | None:
    if "_mlrs_" in unit_name:
        return unit_name.replace("_mlrs_", "_mlrs_spawn_area_")
    if "_tactical_missile_launcher_" in unit_name:
        return unit_name.replace("_tactical_missile_launcher_", "_tactical_missile_spawn_area_")
    return None


def route_of(site: dict) -> dict | None:
    way = site.get("way")
    points, speeds = [], []
    for name, waypoint in (way.items() if isinstance(way, dict) else []):
        if not isinstance(waypoint, dict) or not is_tm(waypoint.get("tm")):
            continue
        props = waypoint.get("props") or {}
        points.append({"name": name, "world_pos": waypoint["tm"][3][:3], "speed": props.get("speed")})
        if isinstance(props.get("speed"), (int, float)):
            speeds.append(props["speed"])
    if not points:
        return None
    return {"points": points, "closed": bool(site.get("closed_waypoints")),
            "speed": speeds[0] if speeds and len(set(speeds)) == 1 else None}


def direct_unit(site: dict, group: str) -> dict:
    lowered = site["unit_class"].lower()
    role = "radar" if "radar" in lowered else "launcher" if ("launcher" in lowered or "_sam_" in lowered) else "ship" if group == "ships" else "vehicle"
    return {"name": site["unit_class"], "role": role, "world_pos": site.get("_at", site["tm"][3][:3]),
            "threat_range": site.get("threatSearchRad"), "lifecycle": {"kind": "none"}, "unplaced": False}


def unit_role(setting: dict, slot_kind: str) -> str:
    if setting.get("spawnName"):
        return str(setting["spawnName"]).lower()
    if slot_kind == "air_def":
        return "spaa"
    template, name = str(setting.get("templateName", "")).lower(), str(setting.get("name", "")).lower()
    return "radar" if "radar" in name else "spaa" if "spaa" in template else "launcher" if "launcher" in name else "vehicle"


def extract_mission(source: Source, path: str) -> tuple[list[dict], list[dict], dict]:
    """Returns (sites, eras, mission document)."""
    mission = source.load(path)
    if mission is None:
        raise RuntimeError(f"Mission file is missing or unreadable: {path}")
    eras = discover_eras(mission)
    units_root = mission.get("units") if isinstance(mission.get("units"), dict) else {}
    areas = mission.get("areas") if isinstance(mission.get("areas"), dict) else {}

    by_name: dict[str, dict] = {}
    squads: dict[str, list] = {}
    for group in units_root.values():
        for unit in as_list(group):
            if isinstance(unit, dict) and unit.get("name"):
                by_name[unit["name"]] = unit
                members = (unit.get("props") or {}).get("squad_members")
                if isinstance(members, list):
                    squads[unit["name"]] = members

    # Units whose mission-editor position is only a placeholder.
    placed_at: dict[str, list] = {}
    for document in [mission, *(doc for doc, _ in source.imports(mission, None))]:
        triggers = document.get("triggers")
        for trigger in (triggers.values() if isinstance(triggers, dict) else []):
            actions = trigger.get("actions") if isinstance(trigger, dict) else None
            for action in as_list(actions.get("unitSpawnOnObjectGroup") if isinstance(actions, dict) else None):
                if not isinstance(action, dict):
                    continue
                for member, target in zip(squads.get(action.get("object"), []), as_list(action.get("target"))):
                    target_unit = by_name.get(target)
                    if target_unit and is_tm(target_unit.get("tm")):
                        placed_at[member] = target_unit["tm"][3][:3]

    imported: dict[int, list[tuple[str, dict]]] = {}
    rules: dict[int, dict] = {}
    for era in eras:
        rank = era["rank"]
        active = source.imports(mission, rank)
        rules[rank] = spawn_rank_rules([mission, *(doc for doc, _ in active)])
        found = []
        for document, with_units in active:
            groups = document.get("units") if with_units and isinstance(document.get("units"), dict) else {}
            for group, units in groups.items():
                found.extend((group, dict(unit)) for unit in as_list(units) if isinstance(unit, dict) and unit.get("unit_class"))
        imported[rank] = found

    imported_names = {unit.get("name", "") for found in imported.values() for group, unit in found if group == "tankModels"}
    for name in [*by_name, *imported_names]:
        area = areas.get(spawn_area_name(name) or "")
        if isinstance(area, dict) and is_tm(area.get("tm")):
            placed_at[name] = area["tm"][3][:3]

    def spawns(name, rank) -> bool:
        return all(rank_test(condition, rank) for condition in rules[rank].get(name, []))

    def blank(name, unit_class, team, group, position, kind, relocated) -> dict:
        return {"name": name, "unit_class": unit_class, "team": team, "source_group": group, "world_pos": position,
                "runtime_relocated": relocated, "runtime_position_kind": kind, "runways": [], "route": None,
                "units_by_era": {era["id"]: [] for era in eras}, "buildings_by_era": {era["id"]: [] for era in eras}}

    sites: list[dict] = []
    for group, group_units in units_root.items():
        for raw in as_list(group_units):
            if not isinstance(raw, dict) or not raw.get("unit_class") or not is_tm(raw.get("tm")):
                continue
            tm, name = raw["tm"], raw.get("name", "unnamed_site")
            position = placed_at.get(name, tm[3][:3])
            site = blank(name, raw["unit_class"], (raw.get("props") or {}).get("army"), group, position,
                         "random_spawn_area_center" if spawn_area_name(name) else "fixed", position != tm[3][:3])
            category = source.category(list((raw.get("additionalEcsTemplates") or {}).keys()))
            if category:
                site["category"] = category
            site["route"] = route_of(raw)
            layout = source.layout(raw["unit_class"])
            if layout["runway"]:
                site["runways"].append({"start": to_world(tm, layout["runway"]["start"]),
                                        "end": to_world(tm, layout["runway"]["end"]), "width": layout["runway"]["width"]})
            for era in eras:
                rank = era["rank"]
                if group in ("tankModels", "ships"):
                    if spawns(name, rank):
                        site["units_by_era"][era["id"]] = [direct_unit({**raw, "_at": position}, group)]
                    continue
                names, configs = site_templates(raw, rank)
                buildings, taken = [], set()
                for config in configs:
                    if config.get("unit__className"):
                        buildings.append({"name": config["unit__className"], "role": "building", "world_pos": tm[3][:3]})
                if source.category(names) == "assembly_area":
                    wanted = source.values(names, {"assembly_area__tankFactoryUnitName", *STORAGE_SLOTS.values()})
                    if wanted.get("assembly_area__tankFactoryUnitName"):
                        buildings.append({"name": wanted["assembly_area__tankFactoryUnitName"], "role": "building", "world_pos": tm[3][:3]})
                    for index, slot in enumerate(layout["slots"]):
                        building = wanted.get(STORAGE_SLOTS.get(slot["kind"], ""))
                        if building:
                            taken.add(index)
                            buildings.append({"name": building, "role": "building", "world_pos": to_world(tm, slot["pos"])})
                site["buildings_by_era"][era["id"]] = buildings

                free = [(i, slot) for i, slot in enumerate(layout["slots"]) if i not in taken and slot["kind"] not in STORAGE_SLOTS]
                used: set[int] = set()
                placed = []
                components_cache: dict[str, bool] = {}
                site_category = source.category(names)
                for setting in source.spawn_settings(names, site["team"]):
                    try:
                        count = max(0, int(setting.get("count", 1)))
                    except (TypeError, ValueError):
                        count = 1
                    wanted_kind = str(setting.get("spawnName", "")).lower()
                    template = str(setting.get("templateName", ""))
                    if template not in components_cache:
                        components_cache[template] = "restore_unit_by_timer" in source.components([template])
                    life = "none" if not components_cache[template] else \
                        "repair_with_site" if site_category in ("fuel_factory", "ammo_factory", "assembly_area") else "restore"
                    for _ in range(count):
                        slot = next(((i, s) for i, s in free if i not in used and (not wanted_kind or s["kind"] == wanted_kind)), None)
                        if slot:
                            used.add(slot[0])
                        placed.append({
                            "name": setting["name"],
                            "role": unit_role(setting, slot[1]["kind"] if slot else "origin"),
                            "world_pos": to_world(tm, slot[1]["pos"]) if slot else tm[3][:3],
                            "threat_range": setting.get("threatSearchRad"),
                            "lifecycle": {"kind": life},
                            "unplaced": bool(free) and slot is None,
                        })
                site["units_by_era"][era["id"]] = placed
            sites.append(site)

    # Imported ships: the imported file picks the ship per rank, the mission owns its patrol route.
    waypoints = mission.get("wayPoints") if isinstance(mission.get("wayPoints"), dict) else {}
    ship_names = sorted({unit.get("name", "") for found in imported.values() for group, unit in found if group == "ships"})
    for name in ship_names:
        route = waypoints.get(f"{name}_waypoints")
        way = route.get("way") if isinstance(route, dict) else None
        first = next((w for w in (way.values() if isinstance(way, dict) else []) if isinstance(w, dict) and is_tm(w.get("tm"))), None)
        if not first or not any(float(v) for v in first["tm"][3][:3]):
            continue  # formation members without their own route are placed by the game at run time
        site = None
        for era in eras:
            raw = next((unit for group, unit in imported[era["rank"]] if group == "ships" and unit.get("name") == name), None)
            if raw is None:
                continue
            resolved = {**raw, "tm": first["tm"], "way": way, "closed_waypoints": bool(route.get("closed_waypoints"))}
            if site is None:
                site = blank(name, raw["unit_class"], (raw.get("props") or {}).get("army"), "ships", first["tm"][3][:3], "ship_route_start", True)
                site["route"] = route_of(resolved)
                sites.append(site)
            site["units_by_era"][era["id"]] = [direct_unit(resolved, "ships")]

    # Imported rocket artillery and missile launchers appear inside their spawn areas.
    for name in sorted(imported_names):
        if name in by_name or name not in placed_at:
            continue
        site = None
        for era in eras:
            raw = next((unit for group, unit in imported[era["rank"]] if group == "tankModels" and unit.get("name") == name), None)
            if raw is None or not is_tm(raw.get("tm")):
                continue
            if site is None:
                site = blank(name, raw["unit_class"], (raw.get("props") or {}).get("army"), "tankModels", placed_at[name], "random_spawn_area_center", True)
                sites.append(site)
            if spawns(name, era["rank"]):
                site["units_by_era"][era["id"]] = [direct_unit({**raw, "_at": placed_at[name]}, "tankModels")]
    return sites, eras, mission


def mission_extras(mission: dict, sites: list[dict]) -> dict:
    """Things the mission states directly: launcher areas and bomber entry points."""
    areas = mission.get("areas") if isinstance(mission.get("areas"), dict) else {}

    def area(name):
        value = areas.get(name)
        if not isinstance(value, dict) or not is_tm(value.get("tm")):
            return None
        tm = value["tm"]
        # An area is a sphere whose size is the length of its x axis.
        radius = (tm[0][0] ** 2 + tm[0][1] ** 2 + tm[0][2] ** 2) ** 0.5
        return {"name": name, "center": tm[3][:3], "radius": round(radius, 1)}

    mobile = []
    for site in sites:
        spawn = area(spawn_area_name(site["name"]) or "")
        if not spawn:
            continue
        moves = [a for a in (area(n) for n in sorted(areas) if n.startswith(f"{site['name']}__move_area_")) if a]
        mobile.append({"name": site["name"], "team": site["team"],
                       "unit_type": "mlrs" if "_mlrs_" in site["name"] else "tbm", "spawn_area": spawn, "move_areas": moves})
    bombers = []
    for name in sorted(areas):
        # numbered areas are entry points; the others are helper distances
        if "nuclear_strategic_bomber_spawn" in name and name.rsplit("_", 1)[-1].isdigit() and is_tm(areas[name].get("tm")):
            team = 1 if name.startswith("t1_") else 2 if name.startswith("t2_") else 0
            bombers.append({"name": name, "team": team, "world_pos": areas[name]["tm"][3][:3]})
    return {"mobile_fire_units": mobile, "bomber_spawns": bombers}


DEPOT_CATEGORIES = ("fuel_factory", "ammo_factory", "assembly_area")


def mission_logic(mission: dict, sites: list[dict], capture_radii: dict) -> dict:
    """Who supplies and attacks whom, by the event's "three nearest" rule.

    The game script sends a supply column to one of the three nearest friendly
    vehicle depots, and an armoured column to one of the three nearest enemy
    depots, strongpoints or airfields.
    """
    def distance(a, b):
        return sum((x - y) ** 2 for x, y in zip(a["world_pos"], b["world_pos"])) ** 0.5

    def nearest(origin, pool):
        return sorted(pool, key=lambda site: distance(origin, site))[:3]

    supply, armor = [], []
    for site in sites:
        category = site.get("category")
        if category in ("fuel_factory", "ammo_factory"):
            pool = [s for s in sites if s.get("category") == "assembly_area" and s["team"] == site["team"]]
            supply += [{"factory": site["name"], "assembly": target["name"], "rank_by_distance": rank + 1}
                       for rank, target in enumerate(nearest(site, pool))]
        if category == "assembly_area":
            pool = [s for s in sites if s["team"] != site["team"] and s.get("category") in (*DEPOT_CATEGORIES, "strongpoint", "airfield")]
            armor += [{"assembly": site["name"], "target": target["name"], "rank_by_distance": rank + 1}
                      for rank, target in enumerate(nearest(site, pool))]
    return {
        "sites": [{"name": site["name"], "capture_radius": capture_radii[site["category"]]}
                  for site in sites if site.get("category") in capture_radii],
        "likely_convoy_links": supply,
        "likely_armored_links": armor,
        "mlrs_tbm_targets": [{"name": site["name"]} for site in sites if site.get("category") in (*DEPOT_CATEGORIES, "airfield")],
        **mission_extras(mission, sites),
    }
