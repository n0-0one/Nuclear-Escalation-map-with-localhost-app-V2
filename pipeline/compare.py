"""Development check: compare this project's extraction with a reference
extraction (map_data*.json from the Nuclear Thunder Interactive Map)."""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import extract  # noqa: E402
from datamine import Datamine  # noqa: E402


def close(a, b, tolerance=0.06) -> bool:
    return all(abs(x - y) <= tolerance for x, y in zip(a, b))


def main() -> int:
    datamine, reference = Path(sys.argv[1]), Path(sys.argv[2])
    config = json.loads((Path(__file__).resolve().parent / "config.json").read_text())
    dm = Datamine(datamine)
    source = extract.Source(dm)
    manifest = json.loads((reference / "tools" / "datamine_sources.json").read_text())
    problems = 0
    for variant in manifest["mission_variants"]:
        golden_file = reference / ("map_data.json" if variant["id"] == "standard" else f"map_data_{variant['id']}.json")
        golden = {site["name"]: site for site in json.loads(golden_file.read_text())}
        sites, eras, _ = extract.extract_mission(source, variant["path"])
        mine = {site["name"]: site for site in sites}
        issues = []
        for name in sorted(set(golden) | set(mine)):
            if name not in mine or name not in golden:
                issues.append(f"{name}: only in {'reference' if name in golden else 'ours'}")
                continue
            a, b = mine[name], golden[name]
            for key in ("unit_class", "team", "source_group", "runtime_position_kind"):
                if a.get(key) != b.get(key):
                    issues.append(f"{name}: {key} {a.get(key)!r} != {b.get(key)!r}")
            if a.get("category") != b.get("category"):
                issues.append(f"{name}: category {a.get('category')!r} != {b.get('category')!r}")
            if not close(a["world_pos"], b["world_pos"]):
                issues.append(f"{name}: position differs")
            if len(a["runways"]) != len(b["runways"]) or any(not close(x["start"], y["start"]) or not close(x["end"], y["end"]) for x, y in zip(a["runways"], b["runways"])):
                issues.append(f"{name}: runways differ")
            if bool(a["route"]) != bool(b["route"]) or (a["route"] and len(a["route"]["points"]) != len(b["route"]["points"])):
                issues.append(f"{name}: route differs")
            for kind in ("units_by_era", "buildings_by_era"):
                for era in sorted(set(a[kind]) | set(b[kind])):
                    ours, theirs = a[kind].get(era), b[kind].get(era)
                    if ours is None or theirs is None or len(ours) != len(theirs):
                        issues.append(f"{name}/{era}/{kind}: {None if ours is None else len(ours)} vs {None if theirs is None else len(theirs)}")
                        continue
                    for x, y in zip(ours, theirs):
                        same = x["name"] == y["name"] and x["role"] == y["role"] and close(x["world_pos"], y["world_pos"])
                        if kind == "units_by_era":
                            same = same and x["lifecycle"]["kind"] == y["lifecycle"]["kind"] and x.get("threat_range") == y.get("threat_range") and bool(x.get("unplaced")) == bool(y.get("unplaced"))
                        if not same:
                            issues.append(f"{name}/{era}: {x['name']} {x['role']} vs {y['name']} {y['role']}")
                            break
        units = sum(len(units) for site in sites for units in site["units_by_era"].values())
        print(f"{variant['id']}: {len(sites)} sites, {units} unit placements, eras {[e['id'] for e in eras]}, {len(issues)} differences")
        for issue in issues[:12]:
            print("   ", issue)
        problems += len(issues)
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main())
