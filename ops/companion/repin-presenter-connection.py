#!/usr/bin/env python3
"""Idempotently re-pin the presenter Companion connection to the stable "dev" module version.

Background (#733): Bitfocus Companion stores each connection's module version in the
`instances` table of its config SQLite DB, as `moduleVersionId`. A connection pinned to a
CONCRETE version (e.g. "0.9.0") breaks with a "missing version" error the moment a plugin
deploy bumps the module version, forcing a manual re-pin in the Companion UI. A connection
pinned to the special "dev" id (used for modules loaded from `--extra-module-path`) is stable
across version bumps — the dev module always presents as version id "dev" and auto-reloads.

This tool sets `moduleVersionId="dev"` for every presenter connection, idempotently, so a
version bump never breaks the connection again. It is meant to run as part of the deploy
WHILE the Companion service is STOPPED, so the DB write is clean and Companion reloads the
new value on start.

Only the pure transform functions (`repin_value`, `repin_rows`) contain the logic under test;
the rest is DB/CLI plumbing.
"""
import argparse
import json
import os
import re
import shutil
import sqlite3
import sys
import time

DEFAULT_MODULE_ID = "presenter"
DEFAULT_DEV_VERSION_ID = "dev"


def parse_major_minor(build_str):
    """Extract 'v<major>.<minor>' from a Companion BUILD string like '5.0.3+9703-stable-...'.

    Returns the config sub-directory name (e.g. 'v5.0') or None if unparseable.
    """
    if not build_str:
        return None
    m = re.match(r"\s*(\d+)\.(\d+)", build_str)
    if not m:
        return None
    return "v%s.%s" % (m.group(1), m.group(2))


def repin_value(value_str, module_id=DEFAULT_MODULE_ID, dev_version_id=DEFAULT_DEV_VERSION_ID):
    """Transform one `instances.value` JSON string.

    Returns (new_value_str_or_None, changed_bool, old_version_or_None).
    Only a connection instance whose moduleId matches and whose moduleVersionId is not
    already the dev id is changed; everything else is left untouched (new_value None).
    A non-JSON / non-dict value is skipped (with a warning) rather than aborting the run.
    """
    try:
        d = json.loads(value_str)
    except (ValueError, TypeError) as exc:
        print("WARNING: skipping row with non-JSON value: %s" % exc, file=sys.stderr)
        return None, False, None
    if not isinstance(d, dict):
        print("WARNING: skipping row whose value is not a JSON object", file=sys.stderr)
        return None, False, None
    if d.get("moduleInstanceType") != "connection":
        return None, False, None
    if d.get("moduleId") != module_id:
        return None, False, None
    old_version = d.get("moduleVersionId")
    if old_version == dev_version_id:
        return None, False, old_version  # already pinned to dev — idempotent no-op
    d["moduleVersionId"] = dev_version_id
    return json.dumps(d), True, old_version


def repin_rows(rows, module_id=DEFAULT_MODULE_ID, dev_version_id=DEFAULT_DEV_VERSION_ID):
    """rows: iterable of (id, value_str).

    Returns (updates, matched_ids) where updates=[(id, new_value_str, old_version)] for the
    rows that actually changed, and matched_ids is every presenter-connection id seen
    (changed or already-dev) so the caller can tell "found but already correct" from
    "not found at all".
    """
    updates = []
    matched_ids = []
    for rid, value_str in rows:
        new_value, changed, old_version = repin_value(value_str, module_id, dev_version_id)
        try:
            parsed = json.loads(value_str)
            is_match = isinstance(parsed, dict) and parsed.get("moduleInstanceType") == "connection" and parsed.get("moduleId") == module_id
        except (ValueError, TypeError):
            is_match = False
        if is_match:
            matched_ids.append(rid)
        if changed:
            updates.append((rid, new_value, old_version))
    return updates, matched_ids


def resolve_db_path(config_dir, build_file, explicit_db):
    """Determine the active Companion config DB path.

    Precedence: explicit --db, else v<major>.<minor>/db.sqlite derived from the running
    Companion BUILD, else <config_dir>/db.sqlite. Raises if none exists.
    """
    if explicit_db:
        if not os.path.isfile(explicit_db):
            raise FileNotFoundError("explicit --db not found: %s" % explicit_db)
        return explicit_db
    build_str = None
    if build_file and os.path.isfile(build_file):
        with open(build_file, "r") as fh:
            build_str = fh.read().strip()
    version_dir = parse_major_minor(build_str)
    if version_dir:
        candidate = os.path.join(config_dir, version_dir, "db.sqlite")
        if os.path.isfile(candidate):
            return candidate
        print("WARNING: expected active DB %s not found; falling back" % candidate, file=sys.stderr)
    fallback = os.path.join(config_dir, "db.sqlite")
    if os.path.isfile(fallback):
        return fallback
    raise FileNotFoundError(
        "no Companion config DB found (config_dir=%s, build=%r)" % (config_dir, build_str)
    )


def main(argv=None):
    parser = argparse.ArgumentParser(description="Re-pin presenter Companion connection to 'dev'.")
    parser.add_argument("--config-dir", default="/home/companion/.config/companion-nodejs")
    parser.add_argument("--build-file", default="/opt/companion/BUILD")
    parser.add_argument("--db", default=None, help="explicit db.sqlite path (overrides autodetect)")
    parser.add_argument("--module-id", default=DEFAULT_MODULE_ID)
    parser.add_argument("--dev-version-id", default=DEFAULT_DEV_VERSION_ID)
    parser.add_argument("--dry-run", action="store_true", help="report changes but do not write")
    args = parser.parse_args(argv)

    db_path = resolve_db_path(args.config_dir, args.build_file, args.db)
    print("Using Companion config DB: %s" % db_path)

    con = sqlite3.connect(db_path)
    try:
        cur = con.cursor()
        rows = list(cur.execute("SELECT id, value FROM instances"))
        updates, matched_ids = repin_rows(rows, args.module_id, args.dev_version_id)

        if not matched_ids:
            print("No '%s' connection found in instances table — nothing to re-pin." % args.module_id)
            return 0

        if not updates:
            print("presenter connection(s) already pinned to '%s' — no change (idempotent)." % args.dev_version_id)
            return 0

        for rid, _new_value, old_version in updates:
            print("Re-pinning connection %s: moduleVersionId %r -> %r" % (rid, old_version, args.dev_version_id))

        if args.dry_run:
            print("--dry-run: not writing.")
            return 0

        backup = "%s.repin-bak-%s" % (db_path, time.strftime("%Y%m%d-%H%M%S"))
        shutil.copy2(db_path, backup)
        print("Backed up DB to %s" % backup)

        for rid, new_value, _old in updates:
            cur.execute("UPDATE instances SET value = ? WHERE id = ?", (new_value, rid))
        con.commit()
        print("Re-pinned %d connection(s) to '%s'." % (len(updates), args.dev_version_id))
        return 0
    finally:
        con.close()


if __name__ == "__main__":
    sys.exit(main())
