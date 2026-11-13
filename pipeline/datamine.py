"""Read-only access to a local clone of the public War Thunder datamine.

The clone can be a cheap one made with::

    git clone --filter=blob:none --no-checkout --depth 1 \
        https://github.com/gszabi99/War-Thunder-Datamine datamine

Files are read straight from git objects, so nothing is checked out and only
the blobs we ask for are downloaded.
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

REPOSITORY = "gszabi99/War-Thunder-Datamine"


class Datamine:
    def __init__(self, repo_dir: str | Path, ref: str = "HEAD"):
        self.repo_dir = Path(repo_dir)
        self.ref = ref
        self.commit = self._git("rev-parse", ref).decode().strip()

    def _git(self, *args: str) -> bytes:
        return subprocess.run(
            ["git", "-C", str(self.repo_dir), *args],
            check=True,
            capture_output=True,
        ).stdout

    def ls(self, prefix: str) -> list[str]:
        """Every file path under ``prefix`` (recursive)."""
        out = self._git("ls-tree", "-r", "--name-only", self.commit, "--", prefix)
        return [line for line in out.decode("utf-8").splitlines() if line]

    def exists(self, path: str) -> bool:
        return bool(self.ls(path))

    def read(self, path: str) -> bytes:
        return self._git("show", f"{self.commit}:{path}")

    def read_text(self, path: str) -> str:
        return self.read(path).decode("utf-8", errors="replace")

    def read_json(self, path: str):
        return json.loads(self.read(path))

    def blob_sha(self, path: str) -> str:
        return self._git("rev-parse", f"{self.commit}:{path}").decode().strip()

    def game_version(self) -> str:
        try:
            return self.read_text("version").strip()
        except subprocess.CalledProcessError:
            return "unknown"
