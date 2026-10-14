#!/usr/bin/env python3
"""Self-tests for check_public_bibles.py (#859).

Run: python3 -m unittest discover -s scripts/ci -p 'test_check_public_bibles.py' -v
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import check_public_bibles as cpb  # noqa: E402

REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))

PRIVATE_CHECKOUT = """
      - name: Check out the private Bible modules (#859)
        uses: actions/checkout@v4
        with:
          repository: zbynekdrlik/presenter-bibles
          ssh-key: ${{ secrets.BIBLES_DEPLOY_KEY }}
          path: data/bibles-private
          persist-credentials: false
"""

READS_PRIVATE = """
      - name: Run Playwright tests
        run: npx playwright test
        env:
          PRESENTER_BIBLE_KJV: ${{ github.workspace }}/data/bibles/kjv.usfm.zip
          PRESENTER_BIBLE_SEB: ${{ github.workspace }}/data/bibles-private/seb.bbl.mybible.zip
"""

RELEASE_UPLOAD = """
      - name: Upload release artifact to GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: presenter.tar.gz
"""


def _job(steps: str) -> dict:
    document = yaml.safe_load("jobs:\n  e2e:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n" + steps.strip("\n") + "\n")
    return document


def _violations(steps: str) -> list[cpb.Violation]:
    return cpb.check_workflow("wf.yml", _job(steps))


class PublicDirTest(unittest.TestCase):
    def test_only_the_public_domain_kjv_passes(self) -> None:
        self.assertEqual(cpb.public_dir_violations(["data/bibles/kjv.usfm.zip", "data/libraries/x.pro"]), [])

    def test_a_restricted_module_is_flagged(self) -> None:
        for module in ("seb.bbl.mybible.zip", "rohacek.bbl.mybible.zip", "sevp.obohu.mybible.zip", "milost.bbl.mybible.zip"):
            with self.subTest(module=module):
                found = cpb.public_dir_violations([f"data/bibles/{module}"])
                self.assertEqual([v.file for v in found], [f"data/bibles/{module}"])
                self.assertIn("zbynekdrlik/presenter-bibles", found[0].message)

    def test_any_file_off_the_allow_list_is_flagged(self) -> None:
        found = cpb.public_dir_violations(["data/bibles/new-translation.usfm.zip", "data/bibles/notes.txt"])
        self.assertEqual(len(found), 2)

    def test_a_nested_copy_of_an_allowed_name_is_flagged(self) -> None:
        self.assertEqual(len(cpb.public_dir_violations(["data/bibles/old/kjv.usfm.zip"])), 1)

    def test_a_sibling_directory_is_not_the_public_dir(self) -> None:
        self.assertEqual(cpb.public_dir_violations(["data/bibles-private/seb.bbl.mybible.zip"]), [])


class PrivateDirTest(unittest.TestCase):
    def test_a_tracked_private_module_is_flagged(self) -> None:
        found = cpb.private_dir_violations(["data/bibles-private/seb.bbl.mybible.zip"])
        self.assertEqual([v.file for v in found], ["data/bibles-private/seb.bbl.mybible.zip"])

    def test_nothing_tracked_passes(self) -> None:
        self.assertEqual(cpb.private_dir_violations([]), [])


class TrackedFilesTest(unittest.TestCase):
    """The repository check reads what git TRACKS, not what lies on disk: a
    developer's private clone in data/bibles-private/ is untracked and fine."""

    def _git(self, root: str, *args: str) -> None:
        subprocess.run(["git", "-C", root, *args], check=True, capture_output=True)

    def test_tracked_files_are_read_from_the_index(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            self._git(root, "init", "-q")
            os.makedirs(os.path.join(root, "data", "bibles"))
            for name in ("kjv.usfm.zip", "seb.bbl.mybible.zip", "untracked.bbl.mybible.zip"):
                with open(os.path.join(root, "data", "bibles", name), "wb") as handle:
                    handle.write(b"PK")
            self._git(root, "add", "data/bibles/kjv.usfm.zip", "data/bibles/seb.bbl.mybible.zip")
            tracked = cpb.tracked_files(root, cpb.PUBLIC_DIR)
            self.assertEqual(sorted(tracked), ["data/bibles/kjv.usfm.zip", "data/bibles/seb.bbl.mybible.zip"])
            self.assertEqual([v.file for v in cpb.public_dir_violations(tracked)], ["data/bibles/seb.bbl.mybible.zip"])


class WorkflowTest(unittest.TestCase):
    def test_reading_after_the_private_checkout_passes(self) -> None:
        self.assertEqual(_violations(PRIVATE_CHECKOUT + READS_PRIVATE), [])

    def test_reading_without_the_private_checkout_is_flagged(self) -> None:
        found = _violations(READS_PRIVATE)
        self.assertEqual(len(found), 1)
        self.assertIn("no earlier step checks out", found[0].message)

    def test_reading_before_the_private_checkout_is_flagged(self) -> None:
        found = _violations(READS_PRIVATE + PRIVATE_CHECKOUT)
        self.assertEqual(len(found), 1)

    def test_an_scp_of_a_private_module_needs_the_checkout(self) -> None:
        step = """
      - name: Deploy Bible files
        run: |
          scp data/bibles/kjv.usfm.zip data/bibles-private/seb.bbl.mybible.zip deploy-target:/opt/presenter/bibles/
"""
        self.assertEqual(len(_violations(step)), 1)
        self.assertEqual(_violations(PRIVATE_CHECKOUT + step), [])

    def test_a_comment_naming_the_private_dir_is_not_a_read(self) -> None:
        step = """
      - name: Create release archive
        run: |
          # data/bibles-private is never checked out here (#859)
          cp data/bibles/kjv.usfm.zip release/bibles/
"""
        self.assertEqual(_violations(step + RELEASE_UPLOAD), [])

    def test_job_env_reading_the_private_dir_needs_the_checkout(self) -> None:
        document = yaml.safe_load(
            "jobs:\n  lat:\n    runs-on: self-hosted\n    env:\n      PRESENTER_BIBLE_SEB: data/bibles-private/seb.bbl.mybible.zip\n    steps:\n      - uses: actions/checkout@v4\n"
        )
        self.assertEqual(len(cpb.check_workflow("wf.yml", document)), 1)

    def test_the_checkout_must_drop_its_credentials(self) -> None:
        found = _violations(PRIVATE_CHECKOUT.replace("persist-credentials: false", "persist-credentials: true"))
        self.assertEqual(len(found), 1)
        self.assertIn("persist-credentials: false", found[0].message)
        self.assertEqual(len(_violations(PRIVATE_CHECKOUT.replace("          persist-credentials: false\n", ""))), 1)

    def test_the_checkout_must_land_in_the_gitignored_path(self) -> None:
        found = _violations(PRIVATE_CHECKOUT.replace("path: data/bibles-private", "path: data/bibles"))
        self.assertEqual(len(found), 1)
        self.assertIn("path: data/bibles-private", found[0].message)

    def test_a_release_job_must_not_check_out_the_private_repo(self) -> None:
        found = _violations(PRIVATE_CHECKOUT + RELEASE_UPLOAD)
        self.assertEqual(len(found), 1)
        self.assertIn("GitHub Release", found[0].message)

    def test_a_release_job_must_not_pack_the_private_dir(self) -> None:
        step = """
      - name: Create release archive
        run: cp data/bibles-private/*.zip release/bibles/
"""
        found = _violations(PRIVATE_CHECKOUT + step + RELEASE_UPLOAD)
        self.assertEqual(len(found), 2)

    def test_gh_release_upload_counts_as_publishing(self) -> None:
        step = """
      - name: Upload asset
        run: gh release upload v1 presenter.tar.gz
"""
        self.assertEqual(len(_violations(PRIVATE_CHECKOUT + step)), 1)

    def test_uploading_the_private_dir_as_an_artifact_is_flagged(self) -> None:
        step = """
      - name: Upload traces
        uses: actions/upload-artifact@v4
        with:
          name: traces
          path: |
            test-results/
            data/bibles-private/
"""
        found = _violations(PRIVATE_CHECKOUT + step)
        self.assertEqual(len(found), 1)
        self.assertIn("artifact", found[0].message)

    def test_a_release_job_with_only_the_public_kjv_passes(self) -> None:
        step = """
      - name: Create release archive
        run: cp data/bibles/kjv.usfm.zip release/bibles/
"""
        self.assertEqual(_violations(step + RELEASE_UPLOAD), [])


class RepositoryTest(unittest.TestCase):
    def test_repository_is_clean(self) -> None:
        found = cpb.check_repository(REPO_ROOT)
        self.assertEqual([v.render() for v in found], [])

    def test_main_exits_zero_on_the_repository(self) -> None:
        self.assertEqual(cpb.main(["--root", REPO_ROOT]), 0)


if __name__ == "__main__":
    unittest.main()
