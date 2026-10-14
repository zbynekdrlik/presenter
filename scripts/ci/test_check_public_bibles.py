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

PIN = "7a0e338ede080a118a90bdd9bb4379fea47f63f2"
OTHER_PIN = "0123456789abcdef0123456789abcdef01234567"

PRIVATE_CHECKOUT = """
      - name: Check out the private Bible modules (#859)
        uses: actions/checkout@v4
        with:
          repository: zbynekdrlik/presenter-bibles
          ref: 7a0e338ede080a118a90bdd9bb4379fea47f63f2
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


class PinTest(unittest.TestCase):
    """Invariant 5: every private checkout pins one 40-hex commit."""

    def test_a_checkout_without_a_ref_is_flagged(self) -> None:
        found = _violations(PRIVATE_CHECKOUT.replace(f"          ref: {PIN}\n", ""))
        self.assertEqual(len(found), 1)
        self.assertIn("pin the private modules", found[0].message)

    def test_a_branch_or_short_ref_is_flagged(self) -> None:
        for ref in ("main", "7a0e338", PIN.upper(), PIN + "0"):
            with self.subTest(ref=ref):
                self.assertEqual(len(_violations(PRIVATE_CHECKOUT.replace(PIN, ref))), 1)

    def test_different_pins_across_workflows_are_flagged(self) -> None:
        first = cpb.private_checkouts("a.yml", _job(PRIVATE_CHECKOUT))
        second = cpb.private_checkouts("b.yml", _job(PRIVATE_CHECKOUT.replace(PIN, OTHER_PIN)))
        self.assertEqual([c.ref for c in first + second], [PIN, OTHER_PIN])
        found = cpb.pin_consistency_violations(first + second)
        self.assertEqual(sorted(v.file for v in found), ["a.yml", "b.yml"])
        self.assertIn("2 different pins", found[0].message)

    def test_one_pin_everywhere_passes(self) -> None:
        checkouts = cpb.private_checkouts("a.yml", _job(PRIVATE_CHECKOUT)) + cpb.private_checkouts("b.yml", _job(PRIVATE_CHECKOUT))
        self.assertEqual(cpb.pin_consistency_violations(checkouts), [])

    def test_the_repository_workflows_share_one_pin(self) -> None:
        checkouts: list[cpb.Checkout] = []
        for path in cpb.workflow_paths(REPO_ROOT):
            with open(path, encoding="utf-8") as handle:
                checkouts.extend(cpb.private_checkouts(path, yaml.safe_load(handle)))
        self.assertTrue(checkouts, "no private checkout found in the workflows")
        refs = {checkout.ref for checkout in checkouts}
        self.assertEqual(len(refs), 1, refs)
        self.assertRegex(refs.pop(), cpb.PIN_RE)


class OldPublicPathTest(unittest.TestCase):
    """Invariant 6: SEB/ROHACEK/SEVP/MILOST never point at data/bibles/<module>."""

    def test_a_step_env_pointing_at_the_old_path_is_flagged(self) -> None:
        found = _violations(PRIVATE_CHECKOUT + READS_PRIVATE.replace("data/bibles-private/seb", "data/bibles/seb"))
        self.assertEqual(len(found), 1)
        self.assertIn("PRESENTER_BIBLE_SEB points at the old public path", found[0].message)

    def test_job_and_workflow_env_are_checked(self) -> None:
        document = yaml.safe_load(
            "env:\n  PRESENTER_BIBLE_ROHACEK: data/bibles/rohacek.bbl.mybible.zip\n"
            "jobs:\n  lat:\n    runs-on: self-hosted\n    env:\n      PRESENTER_BIBLE_SEVP: ./data/bibles/sevp.obohu.mybible.zip\n"
            "    steps:\n      - uses: actions/checkout@v4\n"
        )
        found = cpb.check_workflow("wf.yml", document)
        self.assertEqual(len(found), 2)
        self.assertIn("workflow env", found[0].message)
        self.assertIn("[lat] env", found[1].message)

    def test_a_run_script_assignment_is_checked(self) -> None:
        step = """
      - name: Ingest
        run: |
          export PRESENTER_BIBLE_MILOST="$PWD/data/bibles/milost.bbl.mybible.zip"
          ./target/release/ingest_bibles
"""
        found = _violations(PRIVATE_CHECKOUT + step)
        self.assertEqual(len(found), 1)
        self.assertIn("PRESENTER_BIBLE_MILOST", found[0].message)

    def test_kjv_and_the_private_path_pass(self) -> None:
        self.assertEqual(_violations(PRIVATE_CHECKOUT + READS_PRIVATE), [])


class NeedsPrivateTest(unittest.TestCase):
    """Invariant 4 without an explicit data/bibles-private mention."""

    def _step(self, command: str) -> str:
        return f"""
      - name: Run
        run: {command}
"""

    def test_the_playwright_suite_needs_the_checkout(self) -> None:
        for command in (
            "npx playwright test --shard=1/3",
            "npm run test:playwright",
            "npm run test:playwright:headed",
            "npm test",
            "npx playwright test tests/e2e/wasm-bible.spec.ts",
            "node tests/e2e/global-setup.js",
            "./scripts/dev/refresh-dev-data.sh data/libraries",
        ):
            with self.subTest(command=command):
                found = _violations(self._step(command))
                self.assertEqual(len(found), 1)
                self.assertIn("no earlier step checks out", found[0].message)
                self.assertEqual(_violations(PRIVATE_CHECKOUT + self._step(command)), [])

    def test_unrelated_npm_and_playwright_commands_pass(self) -> None:
        for command in ("npm run test:companion", "npx playwright merge-reports --reporter html ./all-blob-reports", "npm run typecheck", "npx playwright install --with-deps chromium"):
            with self.subTest(command=command):
                self.assertEqual(_violations(self._step(command)), [])

    def test_a_local_ingest_with_a_repo_path_needs_the_checkout(self) -> None:
        step = self._step("PRESENTER_BIBLE_KJV=data/bibles/kjv.usfm.zip ./target/release/ingest_bibles")
        found = _violations(step)
        self.assertEqual(len(found), 1)
        self.assertIn("ingest_bibles", found[0].message)

    def test_a_remote_ingest_with_host_paths_passes(self) -> None:
        step = """
      - name: Import Bible translations
        run: |
          ssh deploy-target << 'REMOTE_SCRIPT'
          export PRESENTER_BIBLE_SEB=/opt/presenter/bibles/seb.bbl.mybible.zip
          /opt/presenter/ingest_bibles
          REMOTE_SCRIPT
"""
        self.assertEqual(_violations(step), [])


class WorkflowExtensionTest(unittest.TestCase):
    """Workflows named *.yaml are checked like *.yml."""

    def test_a_yaml_workflow_is_scanned(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            subprocess.run(["git", "-C", root, "init", "-q"], check=True, capture_output=True)
            workflows = os.path.join(root, ".github", "workflows")
            os.makedirs(workflows)
            with open(os.path.join(workflows, "copied.yaml"), "w", encoding="utf-8") as handle:
                handle.write(yaml.safe_dump({"on": "push", "jobs": {"e2e": {"runs-on": "ubuntu-latest", "steps": [{"uses": "actions/checkout@v4"}, {"run": "npx playwright test"}]}}}))
            with open(os.path.join(workflows, "fine.yml"), "w", encoding="utf-8") as handle:
                handle.write("jobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n")
            self.assertEqual([os.path.basename(p) for p in cpb.workflow_paths(root)], ["copied.yaml", "fine.yml"])
            found = cpb.check_repository(root)
            self.assertEqual([v.file for v in found], [os.path.join(".github", "workflows", "copied.yaml")])


class RepositoryTest(unittest.TestCase):
    def test_repository_is_clean(self) -> None:
        found = cpb.check_repository(REPO_ROOT)
        self.assertEqual([v.render() for v in found], [])

    def test_main_exits_zero_on_the_repository(self) -> None:
        self.assertEqual(cpb.main(["--root", REPO_ROOT]), 0)


if __name__ == "__main__":
    unittest.main()
