"""The reusable site deploy (#918): SNV keeps every step it ran, PRBA skips the device steps.

Run: python3 -m unittest discover -s scripts/ci -p 'test_deploy_site_workflow.py' -v
"""

from __future__ import annotations

import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
SITE = ROOT / ".github/workflows/deploy-site.yml"
DEPLOY = ROOT / ".github/workflows/deploy.yml"

# The SNV deploy job's steps before #918 (deploy.yml at d0381c73), in order, plus
# "Ensure the service user" (#929).
SNV_STEPS = [
    "Check out the private Bible modules (#859)",
    "Download build artifacts",
    "Place binaries",
    "Setup SSH for deployment",
    "Ensure the service user",
    "Create deploy directory",
    "Ensure ADB is installed",
    "Ensure NDI SDK and avahi-daemon are installed",
    "Ensure GStreamer + VA-API are installed",
    "Stage binaries on prod (pull from GitHub, service keeps running)",
    "Backup database",
    "Validate database schema",
    "Deploy Bible files",
    "Sync ProPresenter libraries from git",
    "Import ProPresenter (first deploy or schema reset)",
    "Deploy systemd service",
    "Configure TURN credentials",
    "Configure AI backend",
    "Swap staged binaries and restart service",
    "Verify deployment",
    "Post-deploy smoke test",
    "Verify AI status",
    "NDI fan-out self-heal (#768)",
    "Show deployment info",
    "Install & configure cloudflared (skipped if vars unset)",
    "Import Bible translations",
    "Deploy Companion module",
    "Verify effective RUST_LOG (#850 O1)",
]

HARDWARE_STEPS = [
    "Ensure ADB is installed",
    "Ensure NDI SDK and avahi-daemon are installed",
    "Ensure GStreamer + VA-API are installed",
    "Configure TURN credentials",
    "NDI fan-out self-heal (#768)",
    "Deploy Companion module",
]
LIBRARY_STEPS = [
    "Sync ProPresenter libraries from git",
    "Import ProPresenter (first deploy or schema reset)",
]


def load(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def site_steps() -> list[dict]:
    return load(SITE)["jobs"]["deploy"]["steps"]


def runs_for(step: dict, inputs: dict[str, object]) -> bool:
    """Evaluate the few `if:` shapes deploy-site.yml uses for one site's inputs."""
    cond = str(step.get("if", "")).strip()
    if not cond:
        return True
    cond = cond.removeprefix("${{").removesuffix("}}").strip()
    if cond == "inputs.hardware":
        return bool(inputs["hardware"])
    if cond == "inputs.library_sync":
        return bool(inputs["library_sync"])
    if cond == "!inputs.library_sync":
        return not inputs["library_sync"]
    if cond.startswith("inputs.site == '"):
        site = cond.split("'")[1]
        return inputs["site"] == site  # the vars part is set per repo, assumed set
    raise AssertionError(f"unknown if: {cond}")


def call_inputs(job: str) -> dict[str, object]:
    return load(DEPLOY)["jobs"][job]["with"]


class DeploySiteWorkflowTest(unittest.TestCase):
    def names_for(self, job: str) -> list[str]:
        inputs = call_inputs(job)
        return [s["name"] for s in site_steps() if "name" in s and runs_for(s, inputs)]

    def test_snv_runs_exactly_its_old_steps(self) -> None:
        self.assertEqual(self.names_for("deploy"), SNV_STEPS)

    def test_prba_skips_device_and_library_steps(self) -> None:
        names = self.names_for("deploy-prba")
        for step in HARDWARE_STEPS + LIBRARY_STEPS:
            self.assertNotIn(step, names)
        self.assertNotIn("Install & configure cloudflared (skipped if vars unset)", names)

    def test_prba_requires_the_seeded_db_before_any_db_step(self) -> None:
        names = self.names_for("deploy-prba")
        self.assertIn("Require the seeded database", names)
        self.assertLess(names.index("Require the seeded database"), names.index("Backup database"))
        self.assertIn("Configure the PRBA tunnel (skipped if vars unset)", names)

    def test_prba_runs_after_snv(self) -> None:
        needs = load(DEPLOY)["jobs"]["deploy-prba"]["needs"]
        self.assertIn("deploy", needs)

    def test_prba_call_has_no_devices_and_no_song_sync(self) -> None:
        inputs = call_inputs("deploy-prba")
        self.assertIs(inputs["hardware"], False)
        self.assertIs(inputs["library_sync"], False)
        self.assertEqual(inputs["deploy_dir"], "/opt/presenter-prba")

    def test_no_hardcoded_snv_path_or_port_left(self) -> None:
        text = SITE.read_text(encoding="utf-8")
        code = "\n".join(line for line in text.splitlines() if not line.strip().startswith("#"))
        self.assertNotIn("/opt/presenter/", code)
        self.assertNotIn("localhost:80/", code)
        self.assertNotIn("/etc/presenter/", code)

    def test_prba_unit_sets_rust_log_once_and_no_sync_peer(self) -> None:
        unit = (ROOT / "scripts/deploy/presenter-prba.service").read_text(encoding="utf-8")
        self.assertEqual(sum(1 for l in unit.splitlines() if l.startswith("Environment=RUST_LOG=")), 1)
        self.assertNotIn("PRESENTER_SYNC_PEER_URL", unit)
        self.assertIn("Environment=PRESENTER_COMPANION_ENABLED=0", unit)
        self.assertIn("Environment=PRESENTER_PORT=8081", unit)


def step(name: str) -> dict:
    return next(s for s in site_steps() if s.get("name") == name)


def run_of(name: str) -> str:
    return str(step(name)["run"])


def code_lines(text: str) -> str:
    return "\n".join(l for l in text.splitlines() if not l.strip().startswith("#"))


def unit_values(text: str, key: str) -> list[str]:
    """Every value of `key=` in a unit text, in order (a drop-in's later value wins)."""
    return [l.split("=", 1)[1] for l in text.splitlines() if l.startswith(f"{key}=")]


class ServiceUserTest(unittest.TestCase):
    """#929: each site's service runs as its own system user (dstack two-networks.md §3)."""

    def test_snv_runs_as_presenter_and_deploys_as_newlevel(self) -> None:
        inputs = call_inputs("deploy")
        self.assertEqual(inputs["service_user"], "presenter")
        self.assertEqual(inputs["deploy_user"], "newlevel")

    def test_prba_runs_as_its_own_account(self) -> None:
        inputs = call_inputs("deploy-prba")
        self.assertEqual(inputs["service_user"], inputs["deploy_user"])
        self.assertEqual(inputs["service_user"], "presenter-prba")

    def test_service_user_is_a_required_input(self) -> None:
        inp = load(SITE)[True]["workflow_call"]["inputs"]["service_user"]
        self.assertIs(inp["required"], True)

    def test_snv_effective_unit_runs_as_presenter_with_gpu_and_port_80(self) -> None:
        """The committed unit + the drop-in the deploy writes = what systemd runs on SNV."""
        unit = (ROOT / "scripts/deploy/presenter.service").read_text(encoding="utf-8")
        run = run_of("Deploy systemd service")
        self.assertIn("printf '[Service]\\nUser=%s\\nGroup=%s\\n' \"$SERVICE_USER\" \"$SERVICE_USER\"", run)
        self.assertIn("service.d/service-user.conf", run)
        user = call_inputs("deploy")["service_user"]
        effective = unit + f"\n[Service]\nUser={user}\nGroup={user}\n"
        self.assertEqual(unit_values(effective, "User")[-1], "presenter")
        self.assertEqual(unit_values(effective, "Group")[-1], "presenter")
        self.assertEqual(unit_values(effective, "SupplementaryGroups"), ["render video"])
        self.assertEqual(unit_values(effective, "AmbientCapabilities"), ["CAP_NET_BIND_SERVICE"])
        self.assertEqual(unit_values(effective, "CapabilityBoundingSet"), ["CAP_NET_BIND_SERVICE"])
        # HOME of the service user is the deploy dir (ensure_service_user.sh), which must be
        # the unit's writable path — adb writes its key and server port file there.
        self.assertEqual(unit_values(effective, "ReadWritePaths"), [call_inputs("deploy")["deploy_dir"]])
        self.assertEqual(unit_values(effective, "WorkingDirectory"), [call_inputs("deploy")["deploy_dir"]])

    def test_the_service_user_step_runs_the_script_before_anything_touches_the_dir(self) -> None:
        names = [s["name"] for s in site_steps() if "name" in s]
        self.assertLess(names.index("Setup SSH for deployment"), names.index("Ensure the service user"))
        self.assertLess(names.index("Ensure the service user"), names.index("Create deploy directory"))
        run = run_of("Ensure the service user")
        self.assertIn("< scripts/deploy/ensure_service_user.sh", run)
        self.assertIn("${{ inputs.hardware }}", run)

    def test_no_recursive_chown_while_the_old_service_runs(self) -> None:
        """A `chown -R` before the stop would take the database away from the running service."""
        names = [s["name"] for s in site_steps() if "name" in s]
        swap = names.index("Swap staged binaries and restart service")
        for s in site_steps()[: swap + 1]:
            if s.get("name") == "Swap staged binaries and restart service":
                continue
            self.assertNotIn("chown -R", code_lines(str(s.get("run", ""))), s.get("name"))
        run = code_lines(run_of("Swap staged binaries and restart service"))
        stop = run.index("systemctl stop")
        chown = run.index("sudo chown -R ${{ env.SERVICE_USER }}:${{ env.SERVICE_USER }}")
        start = run.index("systemctl start")
        self.assertLess(stop, chown)
        self.assertLess(chown, start)
        self.assertIn("sudo mv -f staging.new/presenter-server", run)

    def test_deploy_user_never_writes_into_the_deploy_dir_itself(self) -> None:
        """Only staging.new is the deploy user's; every other write goes through sudo."""
        for s in site_steps():
            for line in code_lines(str(s.get("run", ""))).splitlines():
                if "${{ env.DEPLOY_DIR }}" not in line:
                    continue
                for verb in ("mkdir -p", "cp ", "mv ", "rm "):
                    idx = line.find(verb)
                    if idx < 0 or "staging.new" in line or "gstreamer-registry" in line:
                        continue
                    self.assertIn("sudo", line[:idx], f"{s.get('name')}: {line.strip()}")

    def test_an_existing_deploy_dir_keeps_its_owner_until_the_swap(self) -> None:
        run = code_lines(run_of("Create deploy directory"))
        self.assertIn("test -d ${{ env.DEPLOY_DIR }} ||", run)
        self.assertNotIn("chown", run)

    def test_staging_dir_belongs_to_the_deploy_user(self) -> None:
        run = run_of("Stage binaries on prod (pull from GitHub, service keeps running)")
        self.assertIn(
            "sudo install -d -o ${{ env.DEPLOY_USER }} -g ${{ env.DEPLOY_USER }} ${{ env.DEPLOY_DIR }}/staging.new",
            run,
        )

    def test_database_writers_run_as_the_service_user(self) -> None:
        for name in ("Import ProPresenter (first deploy or schema reset)", "Import Bible translations"):
            self.assertIn("sudo -u ${{ env.SERVICE_USER }} env", run_of(name), name)

    def test_files_from_the_runner_land_owned_by_the_service_user(self) -> None:
        bibles = run_of("Deploy Bible files")
        self.assertIn("deploy-target:${{ env.DEPLOY_DIR }}/staging.new/bibles/", bibles)
        self.assertIn("sudo install -m 0644 -o ${{ env.SERVICE_USER }} -g ${{ env.SERVICE_USER }}", bibles)
        libs = run_of("Sync ProPresenter libraries from git")
        self.assertIn('--rsync-path="sudo rsync"', libs)
        self.assertIn("--chown=${{ env.SERVICE_USER }}:${{ env.SERVICE_USER }}", libs)

    def test_verify_fails_unless_the_process_runs_as_the_service_user(self) -> None:
        run = run_of("Verify deployment")
        self.assertIn("systemctl show -p User --value", run)
        self.assertIn('"$PID_USER" != "${{ env.SERVICE_USER }}"', run)
        self.assertIn("exit 1", run[run.index("PID_USER"):])


if __name__ == "__main__":
    unittest.main()
