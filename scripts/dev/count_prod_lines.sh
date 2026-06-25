#!/usr/bin/env bash
# Count PRODUCTION lines of a Rust source file for the file-size gate (#407).
#
# Production lines = everything before the file's INLINE `#[cfg(test)]` test
# module. The boundary is the first `#[cfg(test)]` that is followed by an inline
# module BODY (`mod <name> {`). A `#[cfg(test)] mod tests;` DECLARATION (the test
# code lives in a separate, gate-exempt file) is NOT a boundary — older logic
# stopped at it and silently undercounted everything after, letting 1100-line
# files bypass the 1000-line cap (#407). A file with no inline test module counts
# in full.
#
# Usage: count_prod_lines.sh <file>   # prints the production line count
set -euo pipefail

file="$1"

test_line=$(awk '
  # An attribute line: remember it as a candidate boundary.
  /^[[:space:]]*#\[cfg\(test\)\]/ { cfg = NR; next }
  # Skip blank lines between the attribute and the item it annotates.
  cfg && /^[[:space:]]*$/ { next }
  # First non-blank line after #[cfg(test)]:
  cfg {
    # `mod <name> {` = inline test MODULE body → this is the boundary.
    if ($0 ~ /^[[:space:]]*(pub[[:space:]]+)?mod[[:space:]].*\{/) { print cfg; exit }
    # Anything else (e.g. `mod tests;` external decl, `use ...;`, a test fn) is
    # NOT the inline-module boundary — keep scanning for a later one.
    cfg = 0
  }
' "$file")

if [[ -n "${test_line}" ]]; then
  echo $(( test_line - 1 ))
else
  wc -l < "$file" | tr -d ' '
fi
