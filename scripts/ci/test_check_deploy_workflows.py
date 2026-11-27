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

    def test_a_dev_push_workflow_must_not_restart_companion(self) -> None:
        # 2026-10-14: pipeline.yml (every dev push) stopped and restarted
        # Companion on SNV and PP, ~50 min after the push - here a PP event.
        step = """
      - name: Deploy to companion-pp.lan
        run: |
          ssh companion-pp "sudo systemctl stop companion"
          ssh companion-pp "sudo python3 /tmp/repin-presenter-connection.py --expect-connection"
          python3 ops/companion/repin-presenter-connection.test.py
          ssh companion-pp "sudo systemctl start companion"
"""
        for trigger, expected in (("push:\n    branches: [dev]", 1), ("push:\n    branches: [main]", 0), ("release:\n    types: [published]", 0)):
            with self.subTest(trigger=trigger):
                document = yaml.safe_load(f"on:\n  {trigger}\n" + _normalize(step))
                found = cdw.check_workflow("wf.yml", document)
                self.assertEqual(len(found), expected)
                if expected:
                    self.assertIn("every push to dev", found[0].message)

    def test_the_shared_companion_script_counts_as_a_companion_deploy(self) -> None:
        step = """
      - name: Deploy Companion module
        run: bash scripts/deploy/deploy_companion_module.sh deploy-target SNV
"""
        document = yaml.safe_load("on:\n  push:\n    branches: [dev]\n" + _normalize(step))
        self.assertEqual(len(cdw.check_workflow("wf.yml", document)), 1)

    def test_every_trigger_form_a_dev_push_reaches_is_checked(self) -> None:
        step = """
      - name: Restart
        run: ssh deploy-target "sudo systemctl restart companion"
"""
        cases = (
            ("on: push\n", 1),
            ("on: [push, workflow_dispatch]\n", 1),
            ("on:\n  push:\n", 1),
            ("on:\n  push:\n    branches: ['**']\n", 1),
            ("on:\n  push:\n    branches: ['d*']\n", 1),
            ("on:\n  push:\n    branches-ignore: [main]\n", 1),
            ("on:\n  push:\n    branches-ignore: [dev]\n", 0),
            ("on:\n  push:\n    tags: ['v*']\n    branches: [main]\n", 0),
            ("on:\n  push:\n    tags: ['v*']\n", 0),
            ("on:\n  push:\n    tags: ['v*']\n    branches: [dev]\n", 1),
            ("on:\n  workflow_run:\n    workflows: [Pipeline]\n    types: [completed]\n", 1),
            ("on:\n  workflow_run:\n    workflows: [Pipeline]\n    branches: [main]\n", 0),
            ("on:\n  schedule:\n    - cron: '0 0 * * 0'\n", 0),
            ("on:\n  pull_request:\n", 0),
        )
        for trigger, expected in cases:
            with self.subTest(trigger=trigger):
                self.assertEqual(len(cdw.check_workflow("wf.yml", yaml.safe_load(trigger + _normalize(step)))), expected)

    def test_more_companion_touches_are_seen_on_a_dev_push(self) -> None:
        for command in (
            'ssh deploy-target "sudo systemctl --no-block restart companion"',
            "rsync -az ops/companion/presenter/ deploy-target:/opt/companion-module-dev/presenter/",
            'ssh companion-pp.lan "uptime"',
            'ssh companion-snv "sudo systemctl kill companion"',
        ):
            with self.subTest(command=command):
                step = f"""
      - name: Touch
        run: {command}
"""
                document = yaml.safe_load("on:\n  push:\n    branches: [dev]\n" + _normalize(step))
                self.assertEqual(len(cdw.check_workflow("wf.yml", document)), 1)

    def test_running_the_repin_unit_tests_on_dev_is_not_a_deploy(self) -> None:
        step = """
      - name: Run companion deploy-tool tests (python)
        run: python3 ops/companion/repin-presenter-connection.test.py
"""
        document = yaml.safe_load("on:\n  push:\n    branches: [dev]\n" + _normalize(step))
        self.assertEqual(cdw.check_workflow("wf.yml", document), [])

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


class RunnerImagePinTest(unittest.TestCase):
    """#964: `ubuntu-latest` rolled to Ubuntu 26.04 (glibc 2.43); the release
    binary built there could not start on SNV / PP (Ubuntu 24.04, glibc 2.39).
    A GitHub-hosted job names a fixed image, never the moving `-latest` one."""

    def _wf(self, runs_on: str) -> list[cdw.Violation]:
        doc = "jobs:\n  build:\n    runs-on: " + runs_on + "\n    steps:\n      - run: cargo build\n"
        return cdw.check_workflow("wf.yml", yaml.safe_load(doc))

    def test_ubuntu_latest_is_flagged(self) -> None:
        found = self._wf("ubuntu-latest")
        self.assertEqual(len(found), 1)
        self.assertIn("ubuntu-24.04", found[0].render())

    def test_ubuntu_latest_in_a_list_is_flagged(self) -> None:
        self.assertEqual(len(self._wf("[ubuntu-latest]")), 1)

    def test_pinned_image_and_self_hosted_pass(self) -> None:
        self.assertEqual(self._wf("ubuntu-24.04"), [])
        self.assertEqual(self._wf("self-hosted"), [])



LISTEN_STEP = r"""
      - name: Configure listening transcription
        env:
          LISTEN_SONIOX_API_KEY: ${{ secrets.LISTEN_SONIOX_API_KEY }}
          LISTEN_SONIOX_MODEL: ${{ vars.LISTEN_SONIOX_MODEL }}
          LISTEN_GEMINI_API_KEY: ${{ secrets.LISTEN_GEMINI_API_KEY }}
          LISTEN_GEMINI_MODEL: ${{ vars.LISTEN_GEMINI_MODEL }}
        run: |
          set -euo pipefail
          SECRET_FILE=/etc/presenter/listen.env
          if [ -n "$LISTEN_SONIOX_API_KEY" ] || [ -n "$LISTEN_GEMINI_API_KEY" ]; then
            ssh deploy-target "sudo install -d -m 755 /etc/presenter"
            {
              [ -z "$LISTEN_SONIOX_API_KEY" ] || printf 'PRESENTER_LISTEN_SONIOX_API_KEY=%s\nPRESENTER_LISTEN_SONIOX_MODEL=%s\n' \
                "$LISTEN_SONIOX_API_KEY" "${LISTEN_SONIOX_MODEL:-stt-rt-v6}"
              [ -z "$LISTEN_GEMINI_API_KEY" ] || printf 'PRESENTER_LISTEN_GEMINI_API_KEY=%s\nPRESENTER_LISTEN_GEMINI_MODEL=%s\n' \
                "$LISTEN_GEMINI_API_KEY" "${LISTEN_GEMINI_MODEL:-gemini-3.5-live-translate-preview}"
            } | ssh deploy-target "sudo sh -c 'umask 077 && cat > $SECRET_FILE'"
            echo "Listening keys written to $SECRET_FILE (0600 root-only): soniox=${LISTEN_SONIOX_API_KEY:+set} model=${LISTEN_SONIOX_MODEL:-stt-rt-v6}, gemini=${LISTEN_GEMINI_API_KEY:+set} model=${LISTEN_GEMINI_MODEL:-gemini-3.5-live-translate-preview}"
          else
            ssh deploy-target "sudo rm -f $SECRET_FILE 2>/dev/null || true"
            echo "No listening key set — transcription and guest translation have no key; any stale $SECRET_FILE removed"
          fi
          ssh deploy-target "sudo systemctl daemon-reload"
"""

START_STEP = """
      - name: Start service
        run: ssh deploy-target "sudo systemctl enable ${{ env.SERVICE_NAME }} && sudo systemctl start ${{ env.SERVICE_NAME }}"
"""

DIFFERS = "differs from the canonical step"


class ListenEnvTest(unittest.TestCase):
    """#852 C3-8: a presenter service deploy has the ONE canonical
    "Configure listening transcription" step before the start (0600 via
    `umask 077` over ssh stdin, removed on an empty secret, the key never
    echoed); the secret appears nowhere else in the job."""

    def _messages(self, steps: str) -> list[str]:
        return [v.message for v in _violations(steps)]

    def _mutated(self, old: str, new: str) -> list[str]:
        steps = LISTEN_STEP.replace(old, new)
        self.assertNotEqual(steps, LISTEN_STEP, "the mutation must change the step")
        return self._messages(SAFE_SETUP + steps + START_STEP)

    def test_the_canonical_step_before_the_start_passes(self) -> None:
        self.assertEqual(_violations(SAFE_SETUP + LISTEN_STEP + START_STEP), [])

    def test_every_etc_dir_form_passes(self) -> None:
        for etc in ("/etc/presenter-prba", "${{ env.ETC_DIR }}"):
            with self.subTest(etc=etc):
                steps = LISTEN_STEP.replace("/etc/presenter", etc)
                self.assertEqual(_violations(SAFE_SETUP + steps + START_STEP), [])

    def test_a_service_deploy_without_the_step_is_flagged(self) -> None:
        found = self._messages(SAFE_SETUP + START_STEP)
        self.assertEqual(len(found), 1)
        self.assertIn("no `Configure listening transcription` step", found[0])

    def test_the_step_after_the_start_is_flagged(self) -> None:
        found = self._messages(SAFE_SETUP + START_STEP + LISTEN_STEP)
        self.assertEqual(len(found), 1)
        self.assertIn("AFTER the service start", found[0])

    def test_every_unsafe_edit_of_the_run_is_flagged(self) -> None:
        edits = {
            "no umask": ("umask 077 && cat >", "cat >"),
            "no rm on an empty secret": ("sudo rm -f $SECRET_FILE", "true"),
            "another file removed": ("sudo rm -f $SECRET_FILE", "sudo rm -f /tmp/other"),
            "key echoed": ("          else\n", '            echo "$LISTEN_SONIOX_API_KEY"\n          else\n'),
            "key teed to the log": ("            } | ssh deploy-target \"sudo sh", "            } | tee /dev/stderr | ssh deploy-target \"sudo sh"),
            "gemini key echoed": ("          fi\n", '            echo "$LISTEN_GEMINI_API_KEY"\n          fi\n'),
            "gemini key without its test": ('[ -z "$LISTEN_GEMINI_API_KEY" ] || printf', 'printf'),
            "key in the remote argv": ("cat > $SECRET_FILE'\"", "cat > $SECRET_FILE' && echo $LISTEN_SONIOX_API_KEY\""),
            "key redirected to stderr": ('"${LISTEN_SONIOX_MODEL:-stt-rt-v6}"\n', '"${LISTEN_SONIOX_MODEL:-stt-rt-v6}" >&2\n'),
            "xtrace": ("set -euo pipefail", "set -euxo pipefail"),
            "tee write without umask": ("umask 077 && cat > $SECRET_FILE", "tee $SECRET_FILE"),
            "a backup name": ("SECRET_FILE=/etc/presenter/listen.env", "SECRET_FILE=/etc/presenter/listen.env.bak"),
            "env dump": ("          fi\n", "          fi\n          printenv\n"),
        }
        for label, (old, new) in edits.items():
            with self.subTest(edit=label):
                found = self._mutated(old, new)
                self.assertEqual(len(found), 1, found)
                self.assertIn(DIFFERS, found[0])

    def test_extra_or_renamed_env_is_flagged(self) -> None:
        for label, (old, new) in {
            "second alias": ("          LISTEN_SONIOX_MODEL:", "          KEY2: ${{ secrets.LISTEN_SONIOX_API_KEY }}\n          LISTEN_SONIOX_MODEL:"),
            "renamed": ("LISTEN_SONIOX_API_KEY: ${{", "SONIOX_KEY: ${{"),
            "gemini renamed": ("LISTEN_GEMINI_API_KEY: ${{", "GEMINI_KEY: ${{"),
        }.items():
            with self.subTest(edit=label):
                found = self._mutated(old, new)
                self.assertTrue(any("`env` must be exactly" in message for message in found), found)

    def test_the_secret_in_another_step_is_flagged(self) -> None:
        other = """
      - name: Leak
        run: ssh deploy-target "echo ${{ secrets.LISTEN_SONIOX_API_KEY }} > /etc/presenter/listen.env"
"""
        found = self._messages(SAFE_SETUP + LISTEN_STEP + other + START_STEP)
        self.assertEqual(len(found), 1)
        self.assertIn("is not the `Configure listening transcription` step", found[0])

    def test_the_secret_in_the_job_env_is_flagged(self) -> None:
        document = yaml.safe_load(_normalize(SAFE_SETUP + LISTEN_STEP + START_STEP))
        document["jobs"]["deploy"]["env"] = {"K": "${{ secrets.LISTEN_SONIOX_API_KEY }}"}
        found = [v.message for v in cdw.check_workflow("wf.yml", document)]
        self.assertEqual(len(found), 1)
        self.assertIn("the job's own keys", found[0])

    def test_a_condition_on_the_step_is_flagged(self) -> None:
        found = self._mutated("      - name: Configure listening transcription\n", "      - name: Configure listening transcription\n        if: vars.X != ''\n")
        self.assertTrue(any("no `if:`" in message for message in found), found)

    def test_every_spelling_of_the_secret_in_another_step_is_flagged(self) -> None:
        for spelling in (
            "${{ secrets.listen_soniox_api_key }}",
            "${{ secrets['LISTEN_SONIOX_API_KEY'] }}",
            "${{ secrets.LISTEN_GEMINI_API_KEY }}",
            "${{ secrets['listen_gemini_api_key'] }}",
            "${{ toJSON(secrets) }}",
        ):
            with self.subTest(spelling=spelling):
                other = f"""
      - name: Leak
        run: echo "{spelling}" > /tmp/k
"""
                found = self._messages(SAFE_SETUP + LISTEN_STEP + other + START_STEP)
                self.assertEqual(len(found), 1, found)
                self.assertIn("is not the `Configure listening transcription` step", found[0])

    def test_the_secret_in_workflow_env_or_a_job_container_is_flagged(self) -> None:
        document = yaml.safe_load(_normalize(SAFE_SETUP + LISTEN_STEP + START_STEP))
        document["env"] = {"K": "${{ secrets.LISTEN_SONIOX_API_KEY }}"}
        found = [v.message for v in cdw.check_workflow("wf.yml", document)]
        self.assertEqual(len(found), 1, found)
        self.assertIn("workflow-level keys", found[0])
        document = yaml.safe_load(_normalize(SAFE_SETUP + LISTEN_STEP + START_STEP))
        document["jobs"]["deploy"]["container"] = {"image": "x", "env": {"K": "${{ secrets.LISTEN_SONIOX_API_KEY }}"}}
        found = [v.message for v in cdw.check_workflow("wf.yml", document)]
        self.assertEqual(len(found), 1, found)
        self.assertIn("the job's own keys", found[0])

    def test_a_tracing_shell_around_the_step_is_flagged(self) -> None:
        for where, key, value in (
            ("job", "defaults", {"run": {"shell": "bash -x {0}"}}),
            ("job", "env", {"SHELLOPTS": "xtrace"}),
            ("workflow", "defaults", {"run": {"shell": "bash -x {0}"}}),
            ("workflow", "env", {"BASH_ENV": "/tmp/trace.sh"}),
        ):
            with self.subTest(where=where, key=key):
                document = yaml.safe_load(_normalize(SAFE_SETUP + LISTEN_STEP + START_STEP))
                holder = document["jobs"]["deploy"] if where == "job" else document
                holder[key] = value
                found = [v.message for v in cdw.check_workflow("wf.yml", document)]
                self.assertEqual(len(found), 1, found)
                self.assertIn("traced or replaced shell", found[0])

    def test_a_broken_line_join_is_flagged(self) -> None:
        for label, (old, new) in {
            "space after the backslash": ("MODEL=%s\\n' \\\n                \"$LISTEN_SONIOX", "MODEL=%s\\n' \\ \n                \"$LISTEN_SONIOX"),
            "blank line after the backslash": ("MODEL=%s\\n' \\\n                \"$LISTEN_GEMINI", "MODEL=%s\\n' \\\n\n                \"$LISTEN_GEMINI"),
        }.items():
            with self.subTest(edit=label):
                found = self._mutated(old, new)
                self.assertEqual(len(found), 1, found)
                self.assertIn(DIFFERS, found[0])

    def test_two_etc_dirs_in_one_step_are_flagged(self) -> None:
        found = self._mutated('"sudo install -d -m 755 /etc/presenter"', '"sudo install -d -m 755 /etc/presenter-zz"')
        self.assertEqual(len(found), 1, found)
        self.assertIn("more than one etc dir", found[0])

    def test_continue_on_error_on_the_step_is_flagged(self) -> None:
        found = self._mutated(
            "      - name: Configure listening transcription\n",
            "      - name: Configure listening transcription\n        continue-on-error: true\n",
        )
        self.assertEqual(len(found), 1, found)
        self.assertIn("exactly the keys", found[0])


if __name__ == "__main__":
    unittest.main()
