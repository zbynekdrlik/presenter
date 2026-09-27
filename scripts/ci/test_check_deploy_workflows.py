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
