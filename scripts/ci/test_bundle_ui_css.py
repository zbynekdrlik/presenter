"""Unit tests for bundle_ui_css.py (#850 S8)."""

import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

from bundle_ui_css import bundle, main  # noqa: E402

HTML = """<head>
    <title>Presenter</title>
    <link rel="stylesheet" href="/ui-pkg/b-111.css"/>
    <link rel="stylesheet" href="/ui-pkg/a-222.css"/>
    <!-- fonts -->
<script type="module">
</script>
"""


class BundleTests(unittest.TestCase):
    def test_keeps_the_link_order_not_the_alphabet_and_leaves_one_link(self):
        files = {"b-111.css": ".b{color:red}", "a-222.css": ".a{color:blue}"}
        html, css, name, names = bundle(HTML, files.__getitem__)
        self.assertEqual(css, ".b{color:red}\n.a{color:blue}")
        self.assertEqual(names, ["b-111.css", "a-222.css"])
        self.assertEqual(html.count('rel="stylesheet"'), 1)
        self.assertIn(f'href="/ui-pkg/{name}"', html)
        self.assertLess(html.index(name), html.index("<!-- fonts -->"))

    def test_the_name_follows_the_content(self):
        one = bundle(HTML, lambda _: ".x{}")[2]
        other = bundle(HTML, lambda _: ".y{}")[2]
        self.assertNotEqual(one, other)

    def test_an_open_rule_or_comment_fails_instead_of_swallowing_the_next_file(self):
        for broken in (".x{color:red", "/* open", ".x{}}"):
            with self.assertRaises(SystemExit):
                bundle(HTML, lambda _: broken)

    def test_no_link_or_a_link_of_another_shape_fails(self):
        with self.assertRaises(SystemExit):
            bundle("<head></head>", lambda _: "")
        other = HTML + '<link href="/x.css" rel="stylesheet">\n'
        with self.assertRaises(SystemExit):
            bundle(other, lambda _: ".x{}")

    def test_a_brace_in_a_comment_does_not_count(self):
        html, css, _, _ = bundle(HTML, lambda _: "/* { */.x{}")
        self.assertIn(".x{}", css)
        self.assertEqual(html.count('rel="stylesheet"'), 1)


class MainTests(unittest.TestCase):
    def test_writes_the_bundle_removes_the_sources_and_rewrites_the_index(self):
        with tempfile.TemporaryDirectory() as tmp:
            dist = pathlib.Path(tmp)
            (dist / "index.html").write_text(HTML, encoding="utf-8")
            (dist / "b-111.css").write_text(".b{}", encoding="utf-8")
            (dist / "a-222.css").write_text(".á{}", encoding="utf-8")
            self.assertEqual(main([str(dist)]), 0)
            css = sorted(dist.glob("*.css"))
            self.assertEqual(len(css), 1)
            self.assertEqual(css[0].read_text(encoding="utf-8"), ".b{}\n.á{}")
            self.assertIn(css[0].name, (dist / "index.html").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
