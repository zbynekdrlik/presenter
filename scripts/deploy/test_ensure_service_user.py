"""Tests for ensure_service_user.sh (#929): the service runs as its own system user.

`sudo` is the only boundary replaced (SUDO_CMD → a recorder that logs every privileged call
and runs `test` for real), so no test needs root or touches the host's users.

Run: python3 -m unittest discover -s scripts/deploy -p 'test_ensure_service_user.py' -v
"""

from __future__ import annotations

import getpass
import os
import pwd
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "ensure_service_user.sh"
MISSING_USER = "presenter-no-such-user-929"


class EnsureServiceUserTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.log = root / "sudo.log"
        self.deploy_dir = root / "opt-presenter"
        self.deploy_dir.mkdir()
        self.deploy_home = root / "home-newlevel"
        (self.deploy_home / ".android").mkdir(parents=True)
        self.sudo = root / "sudo-recorder"
        self.sudo.write_text(
            "#!/usr/bin/env bash\nset -euo pipefail\n"
            f'printf "%s\\n" "$*" >> "{self.log}"\n'
            'if [ "$1" = test ]; then exec "$@"; fi\n'
        )
        self.sudo.chmod(0o755)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def run_script(self, user: str, hardware: str = "true", deploy_dir: str | None = None):
        env = dict(os.environ, SUDO_CMD=str(self.sudo), DEPLOY_USER_HOME=str(self.deploy_home))
        return subprocess.run(
            ["bash", str(SCRIPT), user, deploy_dir or str(self.deploy_dir), hardware],
            capture_output=True, text=True, check=False, env=env,
        )

    def calls(self) -> list[str]:
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_missing_user_is_created_as_a_system_user_homed_in_the_deploy_dir(self) -> None:
        result = self.run_script(MISSING_USER, hardware="false")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.calls(),
            [f"useradd --system --user-group --home-dir {self.deploy_dir} --no-create-home "
             f"--shell /usr/sbin/nologin {MISSING_USER}",
             f"test -d {self.deploy_dir}"],
        )

    def test_a_new_site_gets_its_deploy_dir_owned_by_the_service_user(self) -> None:
        new_dir = self.deploy_dir.parent / "opt-presenter-new"
        result = self.run_script(MISSING_USER, hardware="false", deploy_dir=str(new_dir))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.calls()[1:],
            [f"test -d {new_dir}", f"install -d -o {MISSING_USER} -g {MISSING_USER} {new_dir}"],
        )

    def test_existing_account_is_never_modified(self) -> None:
        """PRBA's service user is also its SSH login (home /home/presenter-prba): a usermod
        --home would move its authorized_keys away and lock the deploy out."""
        me = getpass.getuser()
        result = self.run_script(me, hardware="false")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls(), [f"test -d {self.deploy_dir}"])
        self.assertIn(f"home {pwd.getpwnam(me).pw_dir}", result.stdout)

    def test_device_site_refuses_a_user_homed_outside_the_deploy_dir(self) -> None:
        (self.deploy_home / ".android/adbkey").write_text("PRIVATE")
        result = self.run_script(getpass.getuser())
        self.assertEqual(result.returncode, 1)
        self.assertIn("a device site needs", result.stderr)
        self.assertFalse(any(c.startswith("install") for c in self.calls()), self.calls())

    def test_hardware_site_seeds_the_adb_key_the_tvs_already_trust(self) -> None:
        (self.deploy_home / ".android/adbkey").write_text("PRIVATE")
        (self.deploy_home / ".android/adbkey.pub").write_text("PUBLIC")
        result = self.run_script(MISSING_USER)
        self.assertEqual(result.returncode, 0, result.stderr)
        key_dir = self.deploy_dir / ".android"
        src = self.deploy_home / ".android"
        own = f"-o {MISSING_USER} -g {MISSING_USER}"
        self.assertEqual(
            self.calls()[1:],
            [
                f"test -d {self.deploy_dir}",
                f"test -s {key_dir}/adbkey",
                f"install -d -m 0700 {own} {key_dir}",
                f"install -m 0600 {own} {src}/adbkey {key_dir}/adbkey",
                f"install -m 0644 {own} {src}/adbkey.pub {key_dir}/adbkey.pub",
            ],
        )

    def test_a_key_already_seeded_is_never_overwritten(self) -> None:
        (self.deploy_home / ".android/adbkey").write_text("PRIVATE")
        (self.deploy_dir / ".android").mkdir()
        (self.deploy_dir / ".android/adbkey").write_text("ALREADY")
        result = self.run_script(MISSING_USER)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(c.startswith("install") for c in self.calls()), self.calls())
        self.assertIn("already present", result.stdout)

    def test_no_deploy_user_key_warns_loudly(self) -> None:
        result = self.run_script(MISSING_USER)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("::warning::no adb key", result.stdout)
        self.assertFalse(any(c.startswith("install") for c in self.calls()))

    def test_a_site_without_devices_seeds_no_key(self) -> None:
        (self.deploy_home / ".android/adbkey").write_text("PRIVATE")
        result = self.run_script(MISSING_USER, hardware="false")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any("adbkey" in c for c in self.calls()))

    def test_bad_user_name_or_relative_dir_is_refused(self) -> None:
        self.assertEqual(self.run_script("Presenter;rm").returncode, 1)
        self.assertEqual(self.run_script("-presenter").returncode, 1)  # useradd would read an option
        self.assertEqual(self.run_script(MISSING_USER, deploy_dir="opt/presenter").returncode, 1)
        self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()
