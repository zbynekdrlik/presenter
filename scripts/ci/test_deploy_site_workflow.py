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

# The SNV deploy job's steps before #918 (deploy.yml at d0381c73), in order.
SNV_STEPS = [
    "Check out the private Bible modules (#859)",
    "Download build artifacts",
    "Place binaries",
    "Setup SSH for deployment",
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


if __name__ == "__main__":
    unittest.main()
