**/full-test-loop**

Perform a complete build → deploy → test loop before marking work ready.

Steps:
1. Bump `BUILD_SCRIPT_VERSION`, then run `./build-image.sh` from the repo root to generate a fresh image.
2. Deploy that image to the assigned device with `./deploy-image.sh <DEVICE_IP>`.
3. Run `MB_TEST_FOREGROUND=1 ./tests/test-device.sh <DEVICE_IP> --foreground` and wait for Playwright + pytest to finish.

If *any* step fails or needs manual tweaks, fix the issue and restart at step 1. Do not hand off the task until the full cycle completes without interventions.
