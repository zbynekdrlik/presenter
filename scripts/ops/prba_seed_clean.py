#!/usr/bin/env python3
"""Clean a copy of the SNV database for the PRBA seed (#918).

The copy keeps the songs, libraries, playlists, Bibles, sermons and stream-graphics design, and
loses everything that ties it to SNV: its devices (Resolume walls, Android stage TVs, NDI
sources, AbleSet, OSC), runtime state, usage counters, the audit trail and the site settings
(Companion, listening mics, sync peer, the stored AI backend). It fails when any remaining
setting still names a host, URL or IP, so a new site setting cannot slip through unnoticed.

Usage: prba_seed_clean.py <db_path>
"""

from __future__ import annotations

import re
import sqlite3
import sys
from pathlib import Path

CLEARED_TABLES = [
    "resolume_hosts",
    "resolume_push_audit",
    "android_stage_displays",
    "video_sources",
    "ableset_settings",
    "able_set_settings",
    "osc_settings",
    "stage_state",
    "ui_usage",
    "interpreter_tips",
    "settings_audit",
]

# Site settings: the AI backend comes from PRBA's own env file, Companion is off there.
DROPPED_KEY_PREFIXES = ("listen.", "sync.", "feature.companion.", "ai-settings")

ADDRESS = re.compile(r"\b\d{1,3}(\.\d{1,3}){3}\b|https?://|\.lan\b")


def existing_tables(db: sqlite3.Connection) -> set[str]:
    return {row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}


def clean(path: Path) -> int:
    if not path.is_file():
        print(f"error: {path} is not a file", file=sys.stderr)
        return 1
    db = sqlite3.connect(path)
    try:
        tables = existing_tables(db)
        with db:
            for table in CLEARED_TABLES:
                if table in tables:
                    db.execute(f'DELETE FROM "{table}"')
            if "app_settings" in tables:
                for prefix in DROPPED_KEY_PREFIXES:
                    db.execute("DELETE FROM app_settings WHERE key LIKE ?", (prefix + "%",))
        leftovers = []
        if "app_settings" in tables:
            for key, value in db.execute("SELECT key, value FROM app_settings ORDER BY key"):
                if ADDRESS.search(str(value)):
                    leftovers.append(key)
        for key in leftovers:
            print(f"leftover: app_settings.{key}", file=sys.stderr)
        if leftovers:
            return 1
        for table in CLEARED_TABLES:
            if table in tables:
                n = db.execute(f'SELECT COUNT(*) FROM "{table}"').fetchone()[0]
                print(f"cleared {table}={n}")
        for table in ("presentations", "libraries", "playlists", "bible_sermons"):
            if table in tables:
                n = db.execute(f'SELECT COUNT(*) FROM "{table}"').fetchone()[0]
                print(f"kept {table}={n}")
        db.execute("VACUUM")
    finally:
        db.close()
    return 0


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    return clean(Path(argv[1]))


if __name__ == "__main__":
    sys.exit(main(sys.argv))
