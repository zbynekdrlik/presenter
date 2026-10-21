//! The editor's output LIST and output SWITCH (#785, #827, #842, #855): the
//! `ctx` methods that reload `GET /stream/api/outputs` and move the editor to
//! another output. Split out of `mod.rs` (the #742 pattern) to keep it under
//! the file-size gate; the pure helpers they call live in `output_paths.rs`.

use leptos::prelude::*;
use presenter_core::StreamOutputSummary;

use super::{empty_show_state, output_paths, StreamEditorCtx};

impl StreamEditorCtx {
    /// Re-fetch the output list for the header switcher (mount + after a config
    /// change that might add/rename an output). #827: when the slug being edited
    /// is not in the list (deleted / a stale bookmark), switch to the first
    /// listed output, so the select and the editor agree. That switch asks
    /// nothing (#855): the output is gone, so its unsaved element edits can
    /// never be saved, and a declined question would leave the editor on it.
    ///
    /// #842: the list is reloaded on every config change, so only the newest
    /// request may install its response (an older one could lack an output
    /// created since and trigger that fallback), and an unchanged list is not
    /// set again (that would rebuild an open `<select>`).
    pub fn reload_outputs(self) {
        let request = output_paths::next_outputs_request();
        leptos::task::spawn_local(async move {
            let result =
                crate::api::get_json::<Vec<StreamOutputSummary>>("/stream/api/outputs").await;
            if !output_paths::is_latest_outputs_request(request) {
                leptos::logging::log!("stream editor: dropping an outdated output list");
                return;
            }
            match result {
                Ok(list) => {
                    let current = self.output_slug.get_untracked();
                    let fallback = output_paths::fallback_output(&current, &list);
                    if !self
                        .outputs
                        .with_untracked(|shown| output_paths::same_entries(shown, &list))
                    {
                        self.outputs.set(list);
                    }
                    if let Some(slug) = fallback {
                        leptos::logging::log!(
                            "stream editor: output {current:?} does not exist; switching to {slug:?}"
                        );
                        self.enter_output(slug);
                    }
                }
                Err(e) => self.show_toast(&format!("Načítanie výstupov zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Switch the editor to another output (#785): persist the choice
    /// (localStorage + `?output=`), reset the per-output editing state, and
    /// refetch the def + nameplates for the new output. A no-op if unchanged.
    /// Unsaved element edits ask first (#787 reopen); returns whether it switched.
    pub fn switch_output(self, slug: String) -> bool {
        if slug == self.output_slug.get_untracked() || !self.confirm_discard_draft() {
            return false;
        }
        self.enter_output(slug);
        true
    }

    /// The switch itself, with no question: [`Self::switch_output`] after its
    /// dirty guard, and every path that leaves an output because it no longer
    /// exists (the output bar after a DELETE, the list fallback; #855).
    pub(super) fn enter_output(self, slug: String) {
        self.output_slug.set(slug.clone());
        output_paths::persist_output_slug(&slug);
        output_paths::mirror_output_to_url(&slug);
        // Drop the previous output's element/scene selection + draft, and its
        // def (#787): until the new def lands the scene list shows „Načítavam…"
        // and nothing can act on the previous output's scenes.
        self.close_panel();
        self.def.set(None);
        self.active.set(empty_show_state());
        // Refetch everything scoped to the new output.
        self.refresh();
        self.plates.reload_list();
        self.plates.reload_active();
    }
}
