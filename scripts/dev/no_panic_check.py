#!/usr/bin/env python3
"""No-panic gate for quality-check.sh section 10.

CLAUDE.md "Banned Patterns": production code must not contain `unwrap()`,
`expect()` or `panic!` (a panic in the server crashes the service mid-service).
This script finds them in PRODUCTION code only.

Why a script and not an `rg` one-liner: the old section-10 pattern did not even
parse (unclosed group), so `rg` exited 2, the `if` was false and the gate never
fired for months. A correct flat regex would instead flag every inline
`#[cfg(test)] mod tests { .. }`. Telling production from test code needs a
(small) lexer and the module tree, hence this file + `test_no_panic_check.py`
(a seeded self-test that pins the regressions, run by the CI Quality job).

What is matched (on code only, never inside comments / string / char literals):
  `.unwrap()`   `.expect(`   `panic!(` (also `panic![` / `panic!{`)
NOT matched: `unwrap_or`, `unwrap_or_else`, `unwrap_or_default`, `unwrap_err`,
`expect_err`, `unreachable!`, `todo!`, `assert!` ...

Escape hatch (reviewed, use sparingly): a trailing `// allow-panic: <reason>`
comment on the SAME line as the match. The reason must be non-empty.

Scope: `crates/*/src/**/*.rs`. Exempt (each is test / tooling code, not the
shipped server):
  1. `crates/presenter-ui/**`   WASM crate: a panic is a browser-side JS error,
                                not a server crash (CLAUDE.md).
  2. `tests/`, `benches/`, `examples/` directories, `tests.rs`, `build.rs`.
  3. `src/bin/**`               dev / import tooling binaries, not the service.
  4. Files compiled only under test: a parent's `#[cfg(test)] mod <name>;`
     declaration (e.g. `state/stream_fonts/test_fonts.rs`,
     `repository/sync_test_support.rs`, every `*_tests.rs`), plus every file
     below such a module's directory. Resolved from the module tree, so a
     changed test file is recognised even when its (unchanged) parent is not in
     QC_TARGETS.
  5. Feature-gated test helpers: `#![cfg(feature = "mock-integrations")]` /
     `#[cfg(feature = "mock-integrations")] mod ..;` (the `mock_integrations/`
     tree) and items / mods gated on `feature = "test-helpers"`.
  6. Everything from a TOP-LEVEL inline `#[cfg(test)] mod <name> { .. }` to the
     end of the file (the same boundary rule `count_prod_lines.sh` uses;
     clippy's `items_after_test_module` keeps production items out of the
     tail). A NESTED test module / any other `#[cfg(test)]` item (fn, impl,
     use, field, match arm, statement ...) is skipped as that item only.
  7. `#[test]` / `#[tokio::test]` functions (test-only by construction).
  `#[cfg(not(test))]` and `#[cfg(any(test, feature = "other"))]` are PRODUCTION
  (compiled in a release build with the feature) and ARE scanned.

Scoping (same convention as fn_length_check.py, #374 abs/rel no-op class):
  `QC_TARGETS` = newline-separated RELATIVE paths from `git diff --name-only`
  (empty = every file). The whole tree is always parsed (the module tree needs
  the parents) but only violations in target files are reported. Paths are
  compared in relative form, never absolute.

Exit codes: 0 clean, 1 violations found (listed on stdout, one per line as
`file:line: message`), 2 usage / unparsable source / no sources found (the gate
must never silently become a no-op).

Usage: no_panic_check.py <repo_root>
"""

import bisect
import os
import re
import sys
from collections import namedtuple

Violation = namedtuple("Violation", "file line kind text")

# `feature = "<x>"` values that mean "test / mock helper, not shipped".
TEST_FEATURES = {"test-helpers", "mock-integrations"}
EXEMPT_DIRS = {"tests", "benches", "examples"}
EXEMPT_FILES = {"tests.rs", "build.rs"}
MODDIR_SELF_FILES = {"mod.rs", "lib.rs", "main.rs"}

_IDENT = re.compile(r"[A-Za-z0-9_]")
_RAW_STR = re.compile(r'(?:br|cr|r)(#*)"')
_ATTR_START = re.compile(r"#\s*!?\s*\[")
_CFG_ATTR = re.compile(r"^#\s*(!?)\s*\[\s*cfg\s*\((.*)\)\s*\]$", re.S)
_TEST_ATTR = re.compile(
    r"^#\s*\[\s*(?:[A-Za-z_][A-Za-z0-9_]*\s*::\s*)*test\s*(?:\(.*\))?\s*\]$", re.S
)
_ITEM_KW = re.compile(
    r"(?:pub\s*(?:\([^)]*\))?\s+)?"
    r"(?:(?:async|unsafe|const|default|extern(?:\s+\"[^\"]*\")?)\s+)*"
    r"(mod|fn|use|impl|struct|enum|trait|union|const|static|type|let|macro_rules|extern)\b"
)
_MOD_DECL = re.compile(r"\bmod\s+([A-Za-z_][A-Za-z0-9_]*)\s*([;{])")
_ALLOW = re.compile(r"//\s*allow-panic:\s*\S")
_PATTERNS = (
    ("unwrap()", re.compile(r"\.\s*unwrap\s*\(\s*\)")),
    ("expect(", re.compile(r"\.\s*expect\s*\(")),
    ("panic!", re.compile(r"(?<![A-Za-z0-9_])panic\s*!\s*[(\[{]")),
)
_SEMI_ONLY = {"use", "const", "static", "type", "let"}
_BRACE_ITEMS = {"mod", "fn", "impl", "struct", "enum", "trait", "union", "extern", "macro_rules"}


class SourceError(Exception):
    """Source the lexer cannot make sense of; reported with the file name."""


def mask_source(src):
    """Blank comments and string / char literal contents, keep every offset.

    Returns (masked, line_comments) where `masked` has the same length and the
    same newlines as `src` and `line_comments` maps 1-based line -> the text of
    that line's `//` comment (for the allow-panic escape).
    """
    out = list(src)
    comments = {}
    n = len(src)
    i = 0
    line = 1

    def blank(a, b):
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            i += 1
        elif src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j == -1 else j
            comments[line] = src[i:j]
            blank(i, j)
            i = j
        elif src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            if depth:
                raise SourceError(f"line {line}: unterminated block comment")
            line += src.count("\n", i, j)
            blank(i, j)
            i = j
        elif c in "rbc" and (i == 0 or not _IDENT.match(src[i - 1])) and _RAW_STR.match(src, i):
            m = _RAW_STR.match(src, i)
            closing = '"' + m.group(1)
            j = src.find(closing, m.end())
            if j == -1:
                raise SourceError(f"line {line}: unterminated raw string")
            line += src.count("\n", i, j)
            blank(m.end(), j)
            i = j + len(closing)
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            if j >= n:
                raise SourceError(f"line {line}: unterminated string literal")
            line += src.count("\n", i, j)
            blank(i + 1, j)
            i = j + 1
        elif c == "'":
            if src.startswith("\\", i + 1):
                j = src.find("'", i + 3)
                if j == -1:
                    raise SourceError(f"line {line}: unterminated char literal")
                blank(i + 1, j)
                i = j + 1
            elif i + 2 < n and src[i + 2] == "'" and src[i + 1] != "'":
                if src[i + 1] == "\n":
                    line += 1
                blank(i + 1, i + 2)
                i += 3
            else:  # lifetime / loop label
                i += 1
        else:
            i += 1
    return "".join(out), comments


def _parse_cfg(tokens, pos):
    tok = tokens[pos]
    if tok in ("all", "any", "not") and tokens[pos + 1] == "(":
        pos += 2
        args = []
        while tokens[pos] != ")":
            node, pos = _parse_cfg(tokens, pos)
            args.append(node)
            if tokens[pos] == ",":
                pos += 1
        return (tok, args), pos + 1
    if pos + 1 < len(tokens) and tokens[pos + 1] == "=":
        return ("kv", tok, tokens[pos + 2]), pos + 3
    return ("flag", tok), pos + 1


def _is_test_only(node):
    kind = node[0]
    if kind == "flag":
        return node[1] == "test"
    if kind == "kv":
        return node[1] == "feature" and node[2].strip('"') in TEST_FEATURES
    if kind == "all":  # every arm must hold, so ONE test-only arm makes it test-only
        return any(_is_test_only(a) for a in node[1])
    if kind == "any":  # only test-only if every arm is
        return bool(node[1]) and all(_is_test_only(a) for a in node[1])
    return False  # not(..)


def cfg_is_test_only(expr):
    tokens = re.findall(r'"(?:[^"\\]|\\.)*"|[A-Za-z_][A-Za-z0-9_]*|[(),=]', expr)
    try:
        node, pos = _parse_cfg(tokens, 0)
    except IndexError:
        raise SourceError(f"cannot parse cfg expression: {expr!r}") from None
    if pos != len(tokens):
        raise SourceError(f"cannot parse cfg expression: {expr!r}")
    return _is_test_only(node)


def _match_close(masked, open_idx, opener, closer):
    depth = 0
    for k in range(open_idx, len(masked)):
        if masked[k] == opener:
            depth += 1
        elif masked[k] == closer:
            depth -= 1
            if depth == 0:
                return k
    raise SourceError(f"unbalanced {opener}{closer} starting at offset {open_idx}")


def _skip_ws(masked, i):
    while i < len(masked) and masked[i].isspace():
        i += 1
    return i


def _skip_attrs(masked, i):
    """Skip whitespace and outer attributes; return the index of the item."""
    i = _skip_ws(masked, i)
    while True:
        m = _ATTR_START.match(masked, i)
        if not m:
            return i
        i = _skip_ws(masked, _match_close(masked, m.end() - 1, "[", "]") + 1)


def _item_end(masked, start, kind):
    """Index just past the item / field / arm / statement starting at `start`."""
    n = len(masked)
    depth = 0
    i = start
    while i < n:
        c = masked[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            if depth == 0:  # the enclosing scope closes: the item ended before it
                return i
            depth -= 1
            if c == "}" and depth == 0:
                if kind in _BRACE_ITEMS:
                    return i + 1
                if kind not in _SEMI_ONLY:  # expression / field / match arm
                    j = _skip_ws(masked, i + 1)
                    if re.match(r"else\b", masked[j : j + 5]) or masked[j : j + 1] in (".", "?"):
                        i += 1
                        continue
                    if masked[j : j + 1] in (",", ";"):
                        return j + 1
                    return i + 1
        elif depth == 0:
            if c == ";":
                return i + 1
            if c == "," and kind not in _SEMI_ONLY and kind not in _BRACE_ITEMS:
                return i + 1
        i += 1
    raise SourceError(f"unterminated item starting at offset {start}")


FileScan = namedtuple("FileScan", "violations whole_file_test declared_test_mods")


def scan_source(src, rel="<src>"):
    """Scan one file's text; returns FileScan (violations are production-only)."""
    masked, line_comments = mask_source(src)
    n = len(masked)
    dead = []  # (start, end) ranges that are test code
    declared = set()
    whole_file_test = False
    depth = 0
    pos = 0
    cut = None
    try:
        while True:
            m = _ATTR_START.search(masked, pos)
            if not m:
                break
            attr_start = m.start()
            depth += masked.count("{", pos, attr_start) - masked.count("}", pos, attr_start)
            attr_end = _match_close(masked, m.end() - 1, "[", "]") + 1
            attr_text = re.sub(r"\s+", " ", src[attr_start:attr_end]).strip()
            pos = attr_end
            cfg = _CFG_ATTR.match(attr_text)
            is_test_cfg = bool(cfg) and cfg_is_test_only(cfg.group(2))
            is_test_fn = bool(_TEST_ATTR.match(attr_text))
            if not (is_test_cfg or is_test_fn):
                continue
            if cfg and cfg.group(1) == "!":  # inner attribute: gates the whole file
                if depth == 0:
                    whole_file_test = True
                    break
                continue
            item_start = _skip_attrs(masked, attr_end)
            kw = _ITEM_KW.match(masked, item_start)
            kind = kw.group(1) if kw else "expr"
            item_end = _item_end(masked, item_start, kind)
            if kind == "mod":
                decl = _MOD_DECL.match(masked, kw.start(1))
                if decl and decl.group(2) == ";":
                    declared.add(decl.group(1))
                elif decl and depth == 0 and is_test_cfg:
                    cut = attr_start  # top-level inline test module: rest of file
                    break
            dead.append((attr_start, item_end))
            depth += masked.count("{", attr_end, item_end) - masked.count("}", attr_end, item_end)
            pos = max(pos, item_end)
    except SourceError as exc:
        raise SourceError(f"{rel}: {exc}") from None

    if whole_file_test:
        return FileScan([], True, declared)
    if cut is not None:
        dead.append((cut, n))
    live = list(masked)
    for a, b in dead:
        for k in range(a, b):
            if live[k] != "\n":
                live[k] = " "
    live = "".join(live)

    line_starts = [0] + [k + 1 for k, ch in enumerate(src) if ch == "\n"]
    lines = src.split("\n")
    violations = []
    for kind, pat in _PATTERNS:
        for hit in pat.finditer(live):
            line_no = bisect.bisect_right(line_starts, hit.start())
            if _ALLOW.search(line_comments.get(line_no, "")):
                continue
            violations.append(Violation(rel, line_no, kind, lines[line_no - 1].strip()))
    violations.sort(key=lambda v: (v.line, v.kind))
    return FileScan(violations, False, declared)


def _statically_exempt(rel):
    parts = rel.split("/")
    if len(parts) > 1 and parts[1] == "presenter-ui":
        return True
    if any(p in EXEMPT_DIRS for p in parts[:-1]) or parts[-1] in EXEMPT_FILES:
        return True
    return len(parts) > 4 and parts[3] == "bin"


def _module_dir(rel):
    d, name = os.path.split(rel)
    return d if name in MODDIR_SELF_FILES else os.path.join(d, name[: -len(".rs")])


def _collect_sources(root):
    found = []
    crates = os.path.join(root, "crates")
    for dirpath, dirnames, filenames in os.walk(crates):
        dirnames.sort()
        for name in sorted(filenames):
            if not name.endswith(".rs"):
                continue
            rel = os.path.relpath(os.path.join(dirpath, name), root).replace(os.sep, "/")
            parts = rel.split("/")
            if len(parts) >= 4 and parts[2] == "src":
                found.append(rel)
    return found


def scan_tree(root, targets=()):
    """Return (violations, scanned_count, test_only_count) for the repo at `root`."""
    sources = _collect_sources(root)
    if not sources:
        raise SourceError(f"no Rust sources under {root}/crates/*/src - wrong root?")
    known = set(sources)
    scans = {}
    for rel in sources:
        if _statically_exempt(rel):
            continue
        with open(os.path.join(root, rel), "r", encoding="utf-8") as fh:
            scans[rel] = scan_source(fh.read(), rel)

    test_only = set()

    def mark(rel):
        test_only.add(rel)
        prefix = _module_dir(rel) + "/"
        test_only.update(f for f in known if f.startswith(prefix))

    for rel, scan in scans.items():
        if scan.whole_file_test:
            mark(rel)
        base = _module_dir(rel)
        for mod in scan.declared_test_mods:
            for cand in (f"{base}/{mod}.rs", f"{base}/{mod}/mod.rs"):
                if cand in known:
                    mark(cand)

    wanted = set(targets)
    violations = []
    for rel, scan in scans.items():
        if rel in test_only or (wanted and rel not in wanted):
            continue
        violations.extend(scan.violations)
    violations.sort(key=lambda v: (v.file, v.line, v.kind))
    return violations, len(scans), len(test_only)


def main(argv):
    if len(argv) != 2:
        sys.stderr.write("usage: no_panic_check.py <repo_root>\n")
        return 2
    root = argv[1]
    targets = [t for t in os.environ.get("QC_TARGETS", "").split("\n") if t.strip()]
    try:
        violations, scanned, test_only = scan_tree(root, targets)
    except (SourceError, OSError, UnicodeDecodeError) as exc:
        sys.stderr.write(f"no_panic_check: error: {exc}\n")
        return 2
    for v in violations:
        print(f"{v.file}:{v.line}: {v.kind} in production code: {v.text}")
    if violations:
        print(
            f"no_panic_check: {len(violations)} violation(s) - return the error "
            "(`?` / map_err / log + degrade) or, if provably infallible, add a "
            "trailing `// allow-panic: <reason>` (CLAUDE.md Banned Patterns)"
        )
        return 1
    print(f"no_panic_check: OK ({scanned} files checked, {test_only} test-only skipped)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
