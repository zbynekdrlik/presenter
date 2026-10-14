#!/usr/bin/env python3
"""Self-tests for check_deploy_workflows.py (#795).

Run: python3 -m unittest discover -s scripts/ci -p 'test_check_deploy_workflows.py'
"""

from __future__ import annotations

import os
import sys
import unittest

import yaml

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import check_deploy_workflows as cdw  # noqa: E402

SAFE_SETUP = """
      - name: Setup SSH for deployment
        id: ssh_setup
        run: |
          rm -f ~/.ssh/config
          cat > ~/.ssh/config <<EOF
          Host deploy-target
              HostName ${{ env.DEPLOY_HOST }}
          EOF
          ssh-keyscan -T 5 ${{ env.DEPLOY_HOST }} >> ~/.ssh/known_hosts
"""

STALE_SETUP = """
      - name: Setup SSH for deployment
        id: ssh_setup
        run: |
          ssh-keyscan -T 5 ${{ env.DEPLOY_HOST }} >> ~/.ssh/known_hosts
          cat > ~/.ssh/config <<EOF
          Host deploy-target
              HostName ${{ env.DEPLOY_HOST }}
          EOF
"""


def _normalize(steps: str) -> str:
    return "jobs:\n  deploy:\n    runs-on: self-hosted\n    steps:\n" + steps.strip("\n") + "\n"


def _violations(steps: str) -> list[cdw.Violation]:
    return cdw.check_workflow("wf.yml", yaml.safe_load(_normalize(steps)))


class CheckDeployWorkflowsTest(unittest.TestCase):
    def test_safe_setup_and_gated_always_step_passes(self) -> None:
        steps = SAFE_SETUP + """
      - name: Start service
        if: always() && steps.ssh_setup.outcome == 'success'
        run: ssh deploy-target "sudo systemctl start presenter"
"""
        self.assertEqual(_violations(steps), [])

    def test_ungated_always_remote_step_is_flagged(self) -> None:
        steps = SAFE_SETUP + """
      - name: Start service
        if: always()
        run: ssh deploy-target "sudo systemctl start presenter"
"""
        found = _violations(steps)
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0].step, "Start service")
        self.assertIn("steps.ssh_setup.outcome == 'success'", found[0].message)

    def test_failure_and_cancelled_conditions_are_flagged_too(self) -> None:
        for condition in ("failure()", "cancelled()", "${{ always() }}", "!cancelled()"):
            with self.subTest(condition=condition):
                steps = SAFE_SETUP + f"""
      - name: Recover
        if: "{condition}"
        run: scp file deploy-target:/tmp/file
"""
                self.assertEqual(len(_violations(steps)), 1)

    def test_gate_on_a_different_step_id_is_flagged(self) -> None:
        steps = SAFE_SETUP + """
      - name: Start service
        if: always() && steps.other.outcome == 'success'
        run: ssh deploy-target "sudo systemctl start presenter"
"""
        self.assertEqual(len(_violations(steps)), 1)

    def test_default_success_condition_needs_no_gate(self) -> None:
        steps = SAFE_SETUP + """
      - name: Deploy
        run: ssh deploy-target "true"
      - name: Optional
        if: env.X == '1'
        run: ssh deploy-target "true"
"""
        self.assertEqual(_violations(steps), [])

    def test_always_step_not_touching_deploy_target_is_ignored(self) -> None:
        steps = SAFE_SETUP + """
      - name: Cleanup local pid
        if: always()
        run: rm -f /tmp/pid
"""
        self.assertEqual(_violations(steps), [])

    def test_config_written_after_keyscan_is_flagged(self) -> None:
        found = _violations(STALE_SETUP)
        self.assertEqual(len(found), 1)
        self.assertIn("AFTER ssh-keyscan", found[0].message)

    def test_comments_mentioning_keyscan_or_alias_do_not_count(self) -> None:
        commented = SAFE_SETUP.replace(
            "          rm -f ~/.ssh/config\n",
            "          # write the alias before ssh-keyscan runs\n          rm -f ~/.ssh/config\n",
        )
        steps = commented + """
      - name: Local cleanup
        if: always()
        run: |
          # never ssh deploy-target here
          rm -f /tmp/pid
"""
        self.assertEqual(_violations(steps), [])

    def test_multiline_ssh_continuation_counts_as_remote_use(self) -> None:
        steps = SAFE_SETUP + """
      - name: Recover
        if: always()
        run: |
          ssh -o ConnectTimeout=5 \\
            deploy-target "sudo systemctl start presenter"
"""
        self.assertEqual(len(_violations(steps)), 1)

    def test_gate_or_ed_with_status_function_is_flagged(self) -> None:
        steps = SAFE_SETUP + """
      - name: Start service
        if: always() || steps.ssh_setup.outcome == 'success'
        run: ssh deploy-target "sudo systemctl start presenter"
"""
        self.assertEqual(len(_violations(steps)), 1)

    def test_double_quoted_success_gate_is_accepted(self) -> None:
        steps = SAFE_SETUP + """
      - name: Start service
        if: ${{ always() && steps.ssh_setup.outcome == "success" }}
        run: ssh deploy-target "sudo systemctl start presenter"
"""
        self.assertEqual(_violations(steps), [])

    def test_setup_without_id_is_flagged(self) -> None:
        found = _violations(SAFE_SETUP.replace("        id: ssh_setup\n", ""))
        self.assertEqual(len(found), 1)
        self.assertIn("must have an `id`", found[0].message)

    def test_appended_alias_is_flagged(self) -> None:
        found = _violations(SAFE_SETUP.replace("cat > ~/.ssh/config", "cat >> ~/.ssh/config"))
        self.assertEqual(len(found), 1)
        self.assertIn("not appended", found[0].message)

    def test_rsync_destination_counts_as_remote_use(self) -> None:
        steps = SAFE_SETUP + """
      - name: Sync
        if: always()
        run: |
          rsync -az -e "ssh -i ~/.ssh/id_deploy" \\
            data/ \\
            deploy-target:/opt/presenter/
"""
        self.assertEqual(len(_violations(steps)), 1)

    def test_staged_deploy_running_the_old_tools_before_the_swap_is_flagged(self) -> None:
        # #851 D1: deploy.yml ran /opt/presenter/ingest_bibles while the new
        # binary still waited in staging.new/ — the old parser skipped every
        # translation as up-to-date.
        steps = SAFE_SETUP + """
      - name: Import ProPresenter
        run: ssh deploy-target "/opt/presenter/import_propresenter --root /opt/presenter/libraries"
      - name: Import Bible translations
        run: |
          ssh deploy-target << 'REMOTE_SCRIPT'
          PRESENTER_DB_URL=sqlite:///opt/presenter/presenter.db /opt/presenter/ingest_bibles
          REMOTE_SCRIPT
      - name: Swap staged binaries
        run: ssh deploy-target "cd /opt/presenter && mv -f staging.new/presenter-server staging.new/ingest_bibles . && sudo systemctl start presenter"
"""
        found = _violations(steps)
        self.assertEqual(
            [(v.step, "PREVIOUS release" in v.message, "/healthz" in v.message) for v in found],
            [
                ("Import ProPresenter", True, False),
                ("Import Bible translations", True, False),
                ("Import Bible translations", False, True),
            ],
        )

    def test_staged_copy_of_import_propresenter_before_the_swap_passes(self) -> None:
        steps = SAFE_SETUP + """
      - name: Import ProPresenter
        run: ssh deploy-target "${{ env.DEPLOY_DIR }}/staging.new/import_propresenter --root x"
      - name: Swap staged binaries
        run: ssh deploy-target "mv -f staging.new/presenter-server ."
"""
        self.assertEqual(_violations(steps), [])

    def test_staged_ingest_before_the_swap_is_flagged_because_it_migrates(self) -> None:
        steps = SAFE_SETUP + """
      - name: Import Bible translations
        run: ssh deploy-target "/opt/presenter/staging.new/ingest_bibles"
      - name: Swap staged binaries
        run: ssh deploy-target "mv -f staging.new/presenter-server ."
"""
        found = _violations(steps)
        self.assertEqual(len(found), 1)
        self.assertIn("/healthz", found[0].message)

    def test_ingest_right_after_the_swap_without_a_health_wait_is_flagged(self) -> None:
        steps = SAFE_SETUP + """
      - name: Swap staged binaries
        run: ssh deploy-target "mv -f staging.new/presenter-server . && sudo systemctl start presenter"
      - name: Import Bible translations
        run: ssh deploy-target "${{ env.DEPLOY_DIR }}/ingest_bibles"
"""
        self.assertEqual(len(_violations(steps)), 1)

    def test_ingest_after_the_health_wait_passes(self) -> None:
        steps = SAFE_SETUP + """
      - name: Place binaries
        run: |
          cp _artifacts/ingest_bibles target/release/
          chmod +x target/release/ingest_bibles
      - name: Swap staged binaries
        run: ssh deploy-target "mv -f staging.new/presenter-server staging.new/ingest_bibles ."
      - name: Verify deployment
        run: |
          ssh deploy-target << 'REMOTE_SCRIPT'
          for attempt in $(seq 1 30); do curl -sf http://localhost:80/healthz && break; sleep 2; done
          REMOTE_SCRIPT
      - name: Import Bible translations
        run: |
          ssh deploy-target << 'REMOTE_SCRIPT'
          export PRESENTER_DB_URL=sqlite:///opt/presenter/presenter.db
          /opt/presenter/ingest_bibles
          REMOTE_SCRIPT
"""
        self.assertEqual(_violations(steps), [])

    def test_wrapped_ingest_runs_are_seen(self) -> None:
        # Review of a5f2e2a8: only the first word was checked, so a wrapped run
        # before the swap passed both invariants.
        for command in (
            "timeout 600 /opt/presenter/staging.new/ingest_bibles",
            "sudo -u newlevel /opt/presenter/staging.new/ingest_bibles",
            "env A=1 /opt/presenter/staging.new/ingest_bibles",
            "if /opt/presenter/staging.new/ingest_bibles; then echo ok; fi",
            "exec /opt/presenter/staging.new/ingest_bibles",
            "time /opt/presenter/staging.new/ingest_bibles",
            "bash -c '/opt/presenter/staging.new/ingest_bibles'",
            "nice -n 10 /opt/presenter/staging.new/ingest_bibles",
        ):
            with self.subTest(command=command):
                steps = SAFE_SETUP + f"""
      - name: Import Bible translations
        run: |
          ssh deploy-target << 'REMOTE_SCRIPT'
          {command}
          REMOTE_SCRIPT
      - name: Swap staged binaries
        run: ssh deploy-target "mv -f staging.new/presenter-server ."
"""
                found = _violations(steps)
                self.assertEqual([v.step for v in found], ["Import Bible translations"])
                self.assertIn("/healthz", found[0].message)

    def test_more_wrapper_and_ssh_forms_are_seen(self) -> None:
        # Review round 2: an ssh to a host other than the alias dropped the
        # whole command; combined sudo flags, su -c and flock were missed.
        for command in (
            "ssh -i ~/.ssh/id_deploy newlevel@192.0.2.10 /opt/presenter/import_propresenter",
            "sudo -iu newlevel /opt/presenter/import_propresenter",
            "su newlevel -c '/opt/presenter/import_propresenter'",
            "flock /tmp/deploy.lock /opt/presenter/import_propresenter",
            "flock -w 30 /tmp/deploy.lock /opt/presenter/import_propresenter",
            "flock /tmp/deploy.lock -c '/opt/presenter/import_propresenter'",
            "sudo -uroot /opt/presenter/import_propresenter",
            "ssh -oStrictHostKeyChecking=no deploy-target /opt/presenter/import_propresenter",
        ):
            with self.subTest(command=command):
                steps = SAFE_SETUP + f"""
      - name: Import ProPresenter
        run: {command}
      - name: Swap staged binaries
        run: ssh deploy-target "mv -f staging.new/presenter-server ."
"""
                found = _violations(steps)
                self.assertEqual([v.step for v in found], ["Import ProPresenter"])
                self.assertIn("PREVIOUS release", found[0].message)

    def test_the_health_wait_needs_healthz_inside_the_loop(self) -> None:
        swap = SAFE_SETUP + """
      - name: Swap staged binaries
        run: ssh deploy-target "mv -f staging.new/presenter-server . && sudo systemctl start presenter"
"""
        ingest = """
      - name: Import Bible translations
        run: ssh deploy-target "/opt/presenter/ingest_bibles"
"""
        not_a_wait = """
      - name: Peek
        run: |
          echo "waiting for the service"
          curl -sf http://localhost:80/healthz
"""
        self.assertEqual([v.step for v in _violations(swap + not_a_wait + ingest)], ["Import Bible translations"])
        for wait in (
            "for attempt in $(seq 1 30); do curl -sf http://localhost:80/healthz && break; sleep 2; done",
            "until curl -sf http://localhost:80/healthz; do sleep 2; done",
            'ssh deploy-target "until curl -sf http://localhost:80/healthz; do sleep 2; done"',
            "ssh -o ConnectTimeout=5 deploy-target 'for i in 1 2 3; do curl -sf http://localhost:80/healthz && break; done'",
        ):
            with self.subTest(wait=wait):
                steps = swap + f"""
      - name: Verify deployment
        run: |
          {wait}
""" + ingest
                self.assertEqual(_violations(steps), [])

    def test_a_tool_path_held_in_a_variable_is_resolved(self) -> None:
        # Review round 4: deploy.yml already runs the staged server as
        # `NEW_BIN=...` + `"$NEW_BIN"`; the same shape for a tool was invisible.
        for assign, run in (
            ("NEW_INGEST=/opt/presenter/staging.new/ingest_bibles", '"$NEW_INGEST"'),
            ("export TOOL=${{ env.DEPLOY_DIR }}/import_propresenter", "${TOOL} --root x"),
        ):
            with self.subTest(assign=assign):
                steps = SAFE_SETUP + f"""
      - name: Import
        run: |
          ssh deploy-target << 'REMOTE_SCRIPT'
          {assign}
          {run}
          REMOTE_SCRIPT
      - name: Swap staged binaries
        run: ssh deploy-target "mv -f staging.new/presenter-server ."
"""
                self.assertEqual([v.step for v in _violations(steps)], ["Import"])

    def test_relative_tool_runs_resolve_against_the_last_cd(self) -> None:
        old = SAFE_SETUP + """
      - name: Import ProPresenter
        run: ssh deploy-target "cd /opt/presenter && ./import_propresenter --root x"
      - name: Swap staged binaries
        run: ssh deploy-target "mv -f staging.new/presenter-server ."
"""
        found = _violations(old)
        self.assertEqual(len(found), 1)
        self.assertIn("PREVIOUS release", found[0].message)
        staged = old.replace("cd /opt/presenter &&", "cd ${{ env.DEPLOY_DIR }}/staging.new &&")
        self.assertEqual(_violations(staged), [])

    def test_a_single_healthz_request_is_not_the_health_wait(self) -> None:
        steps = SAFE_SETUP + """
      - name: Swap staged binaries
        run: ssh deploy-target "mv -f staging.new/presenter-server . && sudo systemctl start presenter"
      - name: Peek
        run: ssh deploy-target "curl -s http://localhost:80/healthz || true"
      - name: Import Bible translations
        run: ssh deploy-target "/opt/presenter/ingest_bibles"
"""
        self.assertEqual([v.step for v in _violations(steps)], ["Import Bible translations"])

    def test_jobs_without_a_staged_swap_are_not_checked_for_tools(self) -> None:
        # pipeline.yml / release.yml stop the service and copy the new binaries
        # first, so /opt/.../ingest_bibles IS the new binary there.
        steps = SAFE_SETUP + """
      - name: Stop service
        run: ssh deploy-target "sudo systemctl stop presenter"
      - name: Import Bible translations
        run: ssh deploy-target "/opt/presenter-dev/ingest_bibles"
"""
        self.assertEqual(_violations(steps), [])

    def test_repository_workflows_are_clean(self) -> None:
        root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
        workflows = sorted(
            os.path.join(root, ".github", "workflows", name)
            for name in os.listdir(os.path.join(root, ".github", "workflows"))
            if name.endswith(".yml")
        )
        found: list[cdw.Violation] = []
        for path in workflows:
            with open(path, encoding="utf-8") as handle:
                found.extend(cdw.check_workflow(path, yaml.safe_load(handle)))
        self.assertEqual([v.render() for v in found], [])


if __name__ == "__main__":
    unittest.main()
