//! GPU render-node access (#540).
//!
//! The VA-API / NVENC GStreamer plugins can only register their encoder
//! elements if the process can OPEN the DRM render node (`/dev/dri/renderD128`,
//! `root:render 0660`). When it cannot, the `va` plugin registers ZERO elements,
//! `hw_h264_encoder()` returns `None`, and NDI silently never starts — with every
//! driver package correctly installed, which is exactly what made the PP outage
//! (2026-07-12) so hard to read: the startup warning told us to install packages
//! that were already there.
//!
//! Access must therefore be granted by the SERVICE UNIT (`SupplementaryGroups=render`),
//! never inherited by accident. Production had it only because someone was logged in
//! at the physical console and systemd-logind left a per-seat ACL on the node — a
//! reboot without that login would have killed NDI there the same way.

use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;

/// The DRM render node the VA-API / NVENC stack opens to expose its encoders.
pub const RENDER_NODE: &str = "/dev/dri/renderD128";

/// Whether this process can actually reach the GPU (#540).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderNodeAccess {
    /// No render node at all — the host has no usable GPU.
    Missing,
    /// The node exists but this process cannot open it. Almost always a
    /// permission problem: the service user is not in the `render` group and
    /// holds no logind ACL. Encoders will NOT register, however many driver
    /// packages are installed.
    Unopenable,
    /// The node exists and opens — the GPU is reachable.
    Accessible,
}

/// Classify access to the host's render node.
pub fn render_node_access() -> RenderNodeAccess {
    classify_render_node(Path::new(RENDER_NODE))
}

fn classify_render_node(path: &Path) -> RenderNodeAccess {
    match OpenOptions::new().read(true).write(true).open(path) {
        Ok(_) => RenderNodeAccess::Accessible,
        Err(e) if e.kind() == ErrorKind::NotFound => RenderNodeAccess::Missing,
        Err(_) => RenderNodeAccess::Unopenable,
    }
}

/// The actionable half of the "no H264 encoder" startup warning (#540).
///
/// The old message said "install gstreamer1.0-vaapi / intel-media-va-driver" —
/// on PP every one of those packages WAS installed and the real cause was that
/// the service could not open the render node. Naming the cause the host is
/// actually in turns a multi-hour hunt into a one-line fix.
pub fn missing_encoder_hint(access: RenderNodeAccess) -> &'static str {
    match access {
        RenderNodeAccess::Unopenable => {
            "the GPU render node exists but this process cannot OPEN it — the service user \
             lacks render-node access (systemd: SupplementaryGroups=render, or add the user \
             to the `render` group). Driver packages are NOT the problem; the `va` plugin \
             registers no elements without access to the device."
        }
        RenderNodeAccess::Missing => {
            "this host has no DRM render node (/dev/dri/renderD128) — no GPU is available, \
             so NDI WebRTC cannot be hardware-encoded here."
        }
        RenderNodeAccess::Accessible => {
            "the GPU render node is reachable, so the encoder plugins are missing: install \
             Intel VA-API (gstreamer1.0-vaapi + intel-media-va-driver-non-free) OR NVIDIA \
             NVENC (gstreamer1.0-plugins-bad with nvcodec)."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_render_node, missing_encoder_hint, RenderNodeAccess};
    use std::path::{Path, PathBuf};

    fn nonexistent_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "presenter-no-such-render-node-{}",
            std::process::id()
        ))
    }

    #[test]
    fn absent_render_node_is_missing() {
        assert_eq!(
            classify_render_node(&nonexistent_path()),
            RenderNodeAccess::Missing
        );
    }

    #[test]
    fn present_but_unopenable_node_is_unopenable() {
        // A directory exists but can never be opened read-write — the same shape
        // the service hit on PP, where the node existed and open() was refused.
        assert_eq!(
            classify_render_node(&std::env::temp_dir()),
            RenderNodeAccess::Unopenable
        );
    }

    #[test]
    fn openable_node_is_accessible() {
        assert_eq!(
            classify_render_node(Path::new("/dev/null")),
            RenderNodeAccess::Accessible
        );
    }

    #[test]
    fn hint_for_an_unopenable_node_blames_access_not_packages() {
        // The #540 lesson: the old warning sent us hunting for packages that were
        // already installed. The hint for a permission failure must say so.
        let hint = missing_encoder_hint(RenderNodeAccess::Unopenable);
        assert!(hint.contains("SupplementaryGroups=render"));
        assert!(hint.contains("render` group"));
        assert!(hint.contains("NOT the problem"));

        // …and the other two states must NOT give that advice.
        assert!(!missing_encoder_hint(RenderNodeAccess::Missing).contains("SupplementaryGroups"));
        assert!(missing_encoder_hint(RenderNodeAccess::Missing).contains("no GPU"));
        assert!(missing_encoder_hint(RenderNodeAccess::Accessible).contains("gstreamer1.0-vaapi"));
    }
}

#[cfg(test)]
mod unit_file_tests {
    use std::path::Path;

    /// #540 regression guard. Both deployed units run the server as an ordinary
    /// user, and the render node is `root:render 0660` — so a unit that does not
    /// grant the `render` group cannot open the GPU. On PP this registered zero
    /// VA elements and NDI never started; on prod it "worked" only via a
    /// console-login ACL. The grant belongs in the unit, so assert it there.
    #[test]
    fn deployed_units_grant_the_service_user_gpu_render_access() {
        let deploy_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/deploy");

        for unit in ["presenter.service", "presenter-dev.service"] {
            let path = deploy_dir.join(unit);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

            let grants_render = text
                .lines()
                .map(str::trim)
                .filter_map(|line| line.strip_prefix("SupplementaryGroups="))
                .any(|groups| {
                    groups
                        .split([' ', ',', '\t'])
                        .any(|group| group == "render")
                });

            assert!(
                grants_render,
                "{unit} must grant the service user the `render` group \
                 (SupplementaryGroups=render …) — without it the process cannot open \
                 /dev/dri/renderD128, the GStreamer `va` plugin registers no encoder, \
                 and NDI dies silently (#540)"
            );
        }
    }
}
