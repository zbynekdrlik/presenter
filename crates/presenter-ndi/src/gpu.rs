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

#[cfg(test)]
mod tests {
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
