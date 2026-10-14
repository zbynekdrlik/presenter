#!/usr/bin/env python3
"""Self-test for fn_length_check.py (quality-check.sh section 7).

Seeds a throw-away `crates/` tree per test and asserts the measured function
lengths. Run by the CI Quality job:
  python3 -m unittest discover -s scripts/dev -p 'test_fn_length_check.py' -v

It pins:
  * braces inside string / raw-string / char literals and comments do not count
    (a lone "{" in a test string used to make the function run on to the next
    closing brace — a false >120 hard fail, #852 C1a-2);
  * a long function is measured from its `fn` line to its closing brace;
  * QC_TARGETS scoping compares RELATIVE paths (the #374 abs/rel no-op class).
"""

import os
import sys
import tempfile
import textwrap
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import fn_length_check as flc  # noqa: E402

LIB = "crates/demo/src/lib.rs"


def report(source, targets=()):
    """The checker's {"violations", "warnings"} for one seeded lib.rs."""
    with tempfile.TemporaryDirectory() as root:
        path = os.path.join(root, LIB)
        os.makedirs(os.path.dirname(path))
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(textwrap.dedent(source))
        return flc.analyze(root, list(targets), False)


def body(lines):
    """`lines` statement lines, indented like a fn body."""
    return "".join(f"    let v{k} = {k};\n" for k in range(lines))


def long_fn(name, inner, extra=""):
    """A fn of `inner` + 2 lines (+ the `extra` lines, which must keep balance)."""
    return f"fn {name}() {{\n{extra}{body(inner)}}}\n\n"


def flagged(result):
    return sorted((e["fn"], e["length"]) for e in result["violations"] + result["warnings"])


class FnLengthCheckTest(unittest.TestCase):
    def test_a_long_plain_function_is_measured_to_its_closing_brace(self):
        result = report(long_fn("long", 90))
        self.assertEqual(flagged(result), [("long", 92)])
        self.assertEqual(result["violations"], [])

    def test_a_closing_brace_in_a_string_does_not_end_the_function_early(self):
        # A `"}"` literal closed the count early, so a 132-line function was
        # measured as 2 lines and its hard fail was never reported.
        source = long_fn("long", 130, '    let close = "}";\n')
        self.assertEqual(flagged(report(source)), [("long", 133)])

    def test_a_brace_in_a_string_does_not_merge_two_functions(self):
        # #852 C1a-2: `"{oops"` kept the count open, so the checker ran on to a
        # later closing brace — two short functions read as one >120 function.
        source = long_fn("first", 66, '    let raw = "{oops";\n') + long_fn(
            "second", 56, '    let close = "}";\n'
        )
        self.assertEqual(flagged(report(source)), [])

    def test_braces_in_raw_strings_chars_and_comments_do_not_count(self):
        extra = (
            '    let a = r#"{ \"k\": [1, 2 }"#;\n'
            "    let b = '{';\n"
            "    let c = '}';\n"
            "    // a { in a comment\n"
            "    /* and } in a block comment */\n"
            '    let d = format!("{}{{", a);\n'
        )
        source = long_fn("mixed", 120, extra)
        self.assertEqual(flagged(report(source)), [("mixed", 128)])

    def test_generic_functions_are_measured_and_lifetimes_are_not_chars(self):
        # The fn regex needed `(` right after the name, so every generic fn was
        # invisible to the gate.
        source = "fn borrow<'a>(s: &'a str) -> &'a str {\n" + body(85) + "    s\n}\n"
        self.assertEqual(flagged(report(source)), [("borrow", 88)])

    def test_targets_compare_relative_paths(self):
        source = long_fn("scoped", 90)
        self.assertEqual(flagged(report(source, [LIB])), [("scoped", 92)])
        self.assertEqual(flagged(report(source, ["crates/other/src/lib.rs"])), [])


if __name__ == "__main__":
    unittest.main()
