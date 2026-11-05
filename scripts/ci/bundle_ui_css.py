#!/usr/bin/env python3
"""Join the UI's stylesheets into one file, in their index.html order (#850 S8).

On LAN http (HTTP/1.1, six connections per host) fifteen stylesheets take three
rounds before the wasm can start. The shell loads all of them on every page, so
one file costs nothing and the sources stay separate in the repository.

Usage: bundle_ui_css.py <dist dir>   (after Trunk, before the precompression)
"""

import hashlib
import pathlib
import re
import sys

LINK = re.compile(r'[ \t]*<link rel="stylesheet" href="(?P<prefix>[^"]*/)(?P<name>[^"/]+\.css)"\s*/?>\n?')
COMMENT = re.compile(r"/\*.*?\*/", re.DOTALL)


def check_closed(name: str, css: str) -> None:
    """An open comment or rule at a file's end would swallow the next file."""
    code = COMMENT.sub("", css)
    if "/*" in code or code.count("{") != code.count("}"):
        raise SystemExit(f"{name}: unclosed comment or rule, it would swallow the next stylesheet")


def bundle(html: str, read) -> tuple:
    """(the new html with one link, the joined css, the bundle file name, the source names)."""
    links = list(LINK.finditer(html))
    if not links:
        raise SystemExit("index.html links no stylesheet: did Trunk change its output?")
    names = [link["name"] for link in links]
    parts = []
    for name in names:
        css = read(name)
        check_closed(name, css)
        parts.append(css)
    joined = "\n".join(parts)
    digest = hashlib.sha256(joined.encode("utf-8")).hexdigest()[:16]
    bundle_name = f"app-{digest}.css"
    kept = f'    <link rel="stylesheet" href="{links[0]["prefix"]}{bundle_name}"/>\n'
    first = links[0]
    new_html = html[: first.start()] + kept + LINK.sub("", html[first.end() :])
    if 'rel="stylesheet"' in LINK.sub("", new_html):
        raise SystemExit('index.html has a stylesheet link of another shape: bundle it or fix the pattern')
    return new_html, joined, bundle_name, names


def main(argv: list) -> int:
    if len(argv) != 1:
        raise SystemExit("usage: bundle_ui_css.py <dist dir>")
    dist = pathlib.Path(argv[0])
    index = dist / "index.html"
    html, joined, bundle_name, names = bundle(
        index.read_text(encoding="utf-8"),
        lambda name: (dist / name).read_text(encoding="utf-8"),
    )
    (dist / bundle_name).write_text(joined, encoding="utf-8")
    for name in names:
        (dist / name).unlink()
    index.write_text(html, encoding="utf-8")
    print(f"{len(names)} stylesheets -> {bundle_name} ({len(joined.encode('utf-8'))} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
