"""Refresh the data pack. Run by the scheduled workflow; also works locally.

    python pipeline/update.py [--work pipeline/work] [--assets]

Where each part of the data comes from
--------------------------------------
* Straight from the public datamine (gszabi99/War-Thunder-Datamine), read by
  this project's own code: which missions and maps exist, every unit position
  for both side layouts and all BR brackets, convoy links, launcher areas,
  map bounds, mission settings, every vehicle's battle rating, unit names in
  all languages, kill-feed wording and nuclear bomb yields.
* From majorlier's Nuclear Thunder Interactive Map (data files only, none of
  its code is run): weapon and radar ranges, the mission's rule constants,
  and terrain, roads and map pictures. The last three only exist in the
  compiled game level, not in the datamine.

A new map or BR bracket therefore shows up with correct positions the day it
reaches the datamine, even before anyone else has processed it. If the
datamine's event files are newer than the reference ranges and rules, the
manifest lists them under ``pending_game_changes``.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from datamine import Datamine, REPOSITORY  # noqa: E402

PLACEMENTS_REPOSITORY = "majorlier/nuclear-thunder-interactive-map"


def git(*args: str, cwd: Path | None = None) -> str:
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def clone_datamine(target: Path) -> Datamine:
    if not (target / ".git").exists():
        target.parent.mkdir(parents=True, exist_ok=True)
        git("clone", "--filter=blob:none", "--no-checkout", "--depth", "1", f"https://github.com/{REPOSITORY}", str(target))
    else:
        git("fetch", "--filter=blob:none", "--depth", "1", "origin", cwd=target)
        git("update-ref", "HEAD", "FETCH_HEAD", cwd=target)
    return Datamine(target)


def clone_placements(target: Path, with_assets: bool) -> str:
    """Checks out the data files of the placements source; returns its commit."""
    if not (target / ".git").exists():
        target.parent.mkdir(parents=True, exist_ok=True)
        git("clone", "--filter=blob:none", "--no-checkout", "--depth", "1", f"https://github.com/{PLACEMENTS_REPOSITORY}", str(target))
    else:
        git("fetch", "--filter=blob:none", "--depth", "1", "origin", cwd=target)
        git("update-ref", "HEAD", "FETCH_HEAD", cwd=target)
    patterns = ["/*.json", "/tools/datamine_sources.json"]
    if with_assets:
        patterns += ["/*.png", "/*.webp", "/generated/", "/images/portraits/"]
    git("sparse-checkout", "set", "--no-cone", *patterns, cwd=target)
    git("checkout", "--force", "HEAD", cwd=target)
    return git("rev-parse", "HEAD", cwd=target)


def pending_changes(dm: Datamine, placements: Path) -> list[str]:
    """Event files that changed in the datamine after the placements were extracted."""
    lock_file = placements / "datamine-lock.json"
    if not lock_file.exists():
        return []
    lock = json.loads(lock_file.read_text(encoding="utf-8"))
    changed = []
    for source in lock.get("sources", []):
        if not source.get("used_for_generation"):
            continue
        path = source.get("path", "")
        if "nuclear_escalation" not in path:
            continue  # shared tables (names, vehicles) are read fresh by us anyway
        try:
            if dm.blob_sha(path) != source.get("sha"):
                changed.append(path)
        except subprocess.CalledProcessError:
            changed.append(path)
    return sorted(changed)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--work", type=Path, default=HERE / "work")
    parser.add_argument("--out", type=Path, default=HERE.parent / "data")
    parser.add_argument("--assets", action="store_true", help="also rebuild map pictures, terrain and roads")
    args = parser.parse_args()

    dm = clone_datamine(args.work / "datamine")
    print(f"Datamine {dm.commit[:10]} (game {dm.game_version()})")

    # A level that has no picture yet needs the artwork checkout.
    config = json.loads((HERE / "config.json").read_text(encoding="utf-8"))
    missing_art = any(
        not (args.out / "levels" / f"{level.removeprefix('air_')}.webp").exists() for level in config["level_assets"]
    )
    with_assets = args.assets or missing_art
    placements = args.work / "placements"
    placements_commit = clone_placements(placements, with_assets)
    print(f"Placements {placements_commit[:10]}{' with artwork' if with_assets else ''}")

    pending = pending_changes(dm, placements)
    if pending:
        print(f"{len(pending)} event file(s) changed in the datamine since the placements were extracted:")
        for path in pending:
            print("  ", path)

    command = [
        sys.executable, str(HERE / "pack.py"),
        "--raw", str(placements),
        "--datamine", str(args.work / "datamine"),
        "--assets-commit", placements_commit,
        "--out", str(args.out),
        "--pending", json.dumps(pending),
    ]
    if with_assets:
        command += ["--assets", str(placements)]
    return subprocess.run(command).returncode


if __name__ == "__main__":
    raise SystemExit(main())
