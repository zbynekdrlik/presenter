#!/usr/bin/env python3
"""Self-test for no_panic_check.py (quality-check.sh section 10).

Seeds a throw-away `crates/` tree per test and asserts what the gate reports.
Run by the CI Quality job:
  python3 -m unittest discover -s scripts/dev -p 'test_no_panic_check.py' -v

It pins the regressions that matter:
  * the gate FIRES on production unwrap/expect/panic (the old section-10 regex
    did not even parse, so it never fired);
  * it does NOT fire on test code (inline `#[cfg(test)] mod`, a file declared by
    `#[cfg(test)] mod x;`, feature-gated helpers) nor on look-alikes
    (`unwrap_or*`, comments, string literals);
  * QC_TARGETS scoping compares RELATIVE paths (the #374 abs/rel no-op class).
"""

import os
import subprocess
import sys
import tempfile
import textwrap
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import no_panic_check as npc  # noqa: E402

CHECKER = os.path.join(HERE, "no_panic_check.py")
LIB = "crates/demo/src/lib.rs"


class Tree:
    """A seeded repo root under a temp dir."""

    def __init__(self, files):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = self._tmp.name
        for rel, text in files.items():
            path = os.path.join(self.root, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(textwrap.dedent(text))

    def close(self):
        self._tmp.cleanup()


class NoPanicCheckTest(unittest.TestCase):
    def scan(self, files, targets=()):
        tree = Tree(files)
        self.addCleanup(tree.close)
        violations, _, _ = npc.scan_tree(tree.root, targets)
        return violations

    def hits(self, src, rel=LIB):
        return [(v.line, v.kind) for v in self.scan({rel: src})]

    # --- the gate fires on production code -------------------------------------

    def test_production_unwrap_expect_and_panic_are_flagged(self):
        src = """\
            fn a(x: Option<u8>) -> u8 {
                x.unwrap()
            }
            fn b(x: Option<u8>) -> u8 {
                x.expect("boom")
            }
            fn c() {
                panic!("nope");
            }
            """
        self.assertEqual(
            self.hits(src), [(2, "unwrap()"), (5, "expect("), (8, "panic!")]
        )

    def test_multiline_chain_is_flagged_on_the_expect_line(self):
        src = """\
            fn a(b: Builder) -> Resp {
                b.status(200)
                    .body(x)
                    .expect("valid response")
            }
            """
        self.assertEqual(self.hits(src), [(4, "expect(")])

    def test_panic_with_brackets_and_qualified_path_is_flagged(self):
        src = """\
            fn a() {
                std::panic!("x");
                core::panic![];
            }
            """
        self.assertEqual(self.hits(src), [(2, "panic!"), (3, "panic!")])

    # --- look-alikes are not flagged ------------------------------------------

    def test_unwrap_or_family_and_err_variants_are_not_flagged(self):
        src = """\
            fn a(x: Option<u8>, r: Result<u8, u8>) {
                let _ = x.unwrap_or(1);
                let _ = x.unwrap_or_else(|| 2);
                let _ = x.unwrap_or_default();
                let _ = r.unwrap_err();
                let _ = r.expect_err("must fail");
                unreachable!();
            }
            """
        self.assertEqual(self.hits(src), [])

    def test_comments_and_string_literals_are_not_flagged(self):
        src = r'''
            // x.unwrap() in a line comment
            /// doc: call `.expect("x")` or panic!("y")
            /* block: .unwrap() /* nested .expect( */ panic!() */
            fn a() {
                let _ = "text with .unwrap() and panic!(";
                let _ = r#"raw .expect("quoted") string"#;
                let _ = 'x';
                let _ = '"';
                let _ = b'\'';
                let _ = "esc \" .unwrap() still inside";
            }
            fn lifetimes<'a>(x: &'a str) -> &'a str { x }
            '''
        self.assertEqual(self.hits(src), [])

    def test_code_after_char_literals_and_lifetimes_is_still_scanned(self):
        src = """\
            fn a<'a>(x: &'a str, y: Option<char>) {
                let q = '"';
                y.unwrap();
            }
            """
        self.assertEqual(self.hits(src), [(3, "unwrap()")])

    # --- the reviewed escape hatch --------------------------------------------

    def test_allow_panic_with_a_reason_on_the_same_line_is_honoured(self):
        src = """\
            fn a(x: Option<u8>) -> u8 {
                x.unwrap() // allow-panic: set two lines above, never None
            }
            """
        self.assertEqual(self.hits(src), [])

    def test_allow_panic_without_a_reason_or_on_another_line_is_not(self):
        src = """\
            fn a(x: Option<u8>) -> u8 {
                x.unwrap() // allow-panic:
            }
            fn b(x: Option<u8>) -> u8 {
                // allow-panic: wrong line
                x.unwrap()
            }
            """
        self.assertEqual(self.hits(src), [(2, "unwrap()"), (6, "unwrap()")])

    # --- test code is exempt ---------------------------------------------------

    def test_inline_cfg_test_module_is_ignored_but_production_above_is_not(self):
        src = """\
            fn prod(x: Option<u8>) -> u8 {
                x.unwrap()
            }

            #[cfg(test)]
            mod tests {
                use super::*;

                #[test]
                fn it_works() {
                    assert_eq!(prod(Some(1)), 1);
                    Some(1).unwrap();
                    None::<u8>.expect("x");
                    panic!("in a test");
                }
            }
            """
        self.assertEqual(self.hits(src), [(2, "unwrap()")])

    def test_file_declared_by_cfg_test_mod_is_ignored_with_its_children(self):
        files = {
            LIB: """\
                #[cfg(test)]
                mod helpers;
                mod real;
                """,
            "crates/demo/src/helpers.rs": "fn t() { Some(1).unwrap(); }\n",
            "crates/demo/src/helpers/deeper.rs": "fn t() { Some(1).unwrap(); }\n",
            "crates/demo/src/real.rs": "fn p() { Some(1).unwrap(); }\n",
        }
        self.assertEqual(
            [v.file for v in self.scan(files)], ["crates/demo/src/real.rs"]
        )

    def test_cfg_test_mod_in_a_non_mod_rs_file_resolves_to_its_sibling_dir(self):
        files = {
            LIB: "mod router;\n",
            "crates/demo/src/router.rs": """\
                #[cfg(test)]
                mod tests;
                #[cfg(test)]
                mod stream_tests;
                fn p() {}
                """,
            "crates/demo/src/router/tests.rs": "fn t() { Some(1).unwrap(); }\n",
            "crates/demo/src/router/stream_tests.rs": "fn t() { Some(1).unwrap(); }\n",
            "crates/demo/src/router/other.rs": "fn p() { Some(1).unwrap(); }\n",
        }
        self.assertEqual(
            [v.file for v in self.scan(files)], ["crates/demo/src/router/other.rs"]
        )

    def test_mod_dir_form_of_a_declared_test_file_is_ignored(self):
        files = {
            LIB: "#[cfg(test)]\nmod fixtures;\n",
            "crates/demo/src/fixtures/mod.rs": "fn t() { Some(1).unwrap(); }\n",
        }
        self.assertEqual(self.scan(files), [])

    def test_nested_cfg_test_items_are_skipped_but_following_code_is_not(self):
        src = """\
            struct S;
            impl S {
                #[cfg(test)]
                fn helper(&self) -> u8 {
                    Some(1).unwrap()
                }
                #[cfg(test)]
                pub(crate) fn two<A, B>(&self, a: A, b: B) -> u8 {
                    Some(2).expect("two")
                }
                fn prod(&self) -> u8 {
                    Some(3).unwrap()
                }
            }
            #[cfg(test)]
            use crate::x::{a, b};
            #[cfg(test)]
            impl S {
                fn t(&self) { Some(1).unwrap(); }
            }
            enum E {
                Real,
                #[cfg(test)]
                Fake(u8),
            }
            fn m(e: E) -> u8 {
                match e {
                    E::Real => 1,
                    #[cfg(test)]
                    E::Fake(x) => Some(x).unwrap(),
                    #[cfg(test)]
                    E::Other => {
                        Some(1).expect("arm")
                    }
                }
            }
            fn tail() {
                None::<u8>.unwrap();
            }
            """
        self.assertEqual(self.hits(src), [(12, "unwrap()"), (38, "unwrap()")])

    def test_test_attribute_functions_are_skipped(self):
        src = """\
            #[test]
            fn t1() { Some(1).unwrap(); }
            #[tokio::test]
            async fn t2() { Some(1).unwrap(); }
            #[tokio::test(flavor = "multi_thread")]
            async fn t3() { Some(1).unwrap(); }
            fn prod() { Some(1).unwrap(); }
            """
        self.assertEqual(self.hits(src), [(7, "unwrap()")])

    def test_not_test_and_any_test_or_feature_are_production(self):
        src = """\
            #[cfg(not(test))]
            fn a() { Some(1).unwrap(); }
            #[cfg(any(test, feature = "ai-eval"))]
            fn b() { Some(1).unwrap(); }
            """
        self.assertEqual(self.hits(src), [(2, "unwrap()"), (4, "unwrap()")])

    def test_feature_gated_test_helpers_are_exempt(self):
        files = {
            LIB: """\
                #[cfg(feature = "mock-integrations")]
                pub mod mock_integrations;
                #[cfg(feature = "test-helpers")]
                pub mod test_strip;
                #[cfg(feature = "test-helpers")]
                fn kill_for_test() { Some(1).unwrap(); }
                fn prod() { Some(1).unwrap(); }
                """,
            "crates/demo/src/mock_integrations/mod.rs": "fn m() { Some(1).unwrap(); }\n",
            "crates/demo/src/mock_integrations/resolume.rs": "fn m() { Some(1).unwrap(); }\n",
            "crates/demo/src/test_strip.rs": "fn m() { Some(1).unwrap(); }\n",
        }
        self.assertEqual([(v.file, v.line) for v in self.scan(files)], [(LIB, 7)])

    def test_inner_cfg_attribute_exempts_the_whole_file(self):
        files = {
            "crates/demo/src/mocks/mod.rs": """\
                #![cfg(feature = "mock-integrations")]
                fn m() { Some(1).unwrap(); }
                """,
        }
        self.assertEqual(self.scan(files), [])

    def test_static_path_exemptions(self):
        bad = "fn t() { Some(1).unwrap(); }\n"
        files = {
            "crates/presenter-ui/src/lib.rs": bad,
            "crates/demo/src/tests.rs": bad,
            "crates/demo/src/tests/helpers.rs": bad,
            "crates/demo/src/bin/tool.rs": bad,
            "crates/demo/src/bin/tool/extra.rs": bad,
            "crates/demo/tests/it.rs": bad,
            "crates/demo/benches/b.rs": bad,
            "crates/demo/src/real.rs": bad,
        }
        self.assertEqual(
            [v.file for v in self.scan(files)], ["crates/demo/src/real.rs"]
        )

    # --- scoping (QC_TARGETS, relative paths) ----------------------------------

    def test_targets_restrict_reporting_to_the_listed_relative_files(self):
        bad = "fn p() { Some(1).unwrap(); }\n"
        files = {
            LIB: "mod a;\nmod b;\n",
            "crates/demo/src/a.rs": bad,
            "crates/demo/src/b.rs": bad,
        }
        self.assertEqual(len(self.scan(files)), 2)
        only_a = self.scan(files, targets=["crates/demo/src/a.rs"])
        self.assertEqual([v.file for v in only_a], ["crates/demo/src/a.rs"])
        # An ABSOLUTE target must not silently match (and an unmatched target is no
        # accident to hide): relative form only, like fn_length_check.py.
        tree = Tree(files)
        self.addCleanup(tree.close)
        absolute = os.path.join(tree.root, "crates/demo/src/a.rs")
        none, _, _ = npc.scan_tree(tree.root, [absolute])
        self.assertEqual(none, [])

    def test_changed_test_file_is_exempt_even_when_its_parent_is_not_a_target(self):
        files = {
            LIB: "#[cfg(test)]\nmod helpers;\n",
            "crates/demo/src/helpers.rs": "fn t() { Some(1).unwrap(); }\n",
        }
        self.assertEqual(
            self.scan(files, targets=["crates/demo/src/helpers.rs"]), []
        )

    # --- robustness ------------------------------------------------------------

    def test_no_rust_sources_is_an_error_not_a_silent_pass(self):
        tree = Tree({"README.md": "x\n"})
        self.addCleanup(tree.close)
        with self.assertRaises(npc.SourceError):
            npc.scan_tree(tree.root)

    def test_unterminated_string_is_an_error_not_a_silent_pass(self):
        with self.assertRaises(npc.SourceError):
            self.scan({LIB: 'fn a() { let s = "never closed; }\n'})

    def test_cli_exit_codes_and_output(self):
        dirty = Tree({LIB: "fn a() { Some(1).unwrap(); }\n"})
        clean = Tree({LIB: "fn a() -> u8 { Some(1).unwrap_or(0) }\n"})
        empty = Tree({"README.md": "x\n"})
        for t in (dirty, clean, empty):
            self.addCleanup(t.close)

        def run(tree, targets=""):
            env = dict(os.environ, QC_TARGETS=targets)
            return subprocess.run(
                [sys.executable, CHECKER, tree.root],
                capture_output=True,
                text=True,
                env=env,
                check=False,
            )

        bad = run(dirty)
        self.assertEqual(bad.returncode, 1, bad.stdout + bad.stderr)
        self.assertIn(f"{LIB}:1: unwrap() in production code", bad.stdout)

        good = run(clean)
        self.assertEqual(good.returncode, 0, good.stdout + good.stderr)

        # Scoped to a file with no violation: clean, even though another file is not.
        scoped = run(dirty, targets="crates/demo/src/other.rs")
        self.assertEqual(scoped.returncode, 0, scoped.stdout + scoped.stderr)

        self.assertEqual(run(empty).returncode, 2)


if __name__ == "__main__":
    unittest.main()
