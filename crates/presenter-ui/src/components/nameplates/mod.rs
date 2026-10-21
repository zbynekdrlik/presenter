//! Lower-third nameplates ("menovky"): the plate LIST shared by the stream
//! editor's Menovky tab (#779, #829) and the operator's Menovky view (#871).
//!
//! [`NameplateStore`] holds one output's person-plate list, the plate on air
//! and the live song (the virtual "Pieseň" row), and owns every plate REST call:
//! list / add / edit / delete / reorder / show / hide. [`plate_list`] renders
//! it. Each page adds its own extras around the shared views: the editor its
//! Prehrať preview and "Vytvoriť menovkovú vrstvu"
//! (`stream_editor/editor_nameplates.rs`), the operator the Stream Deck button
//! numbers and the no-layer hint ([`operator_view`]). There is no second copy of
//! the plate logic, the REST paths or the request bodies anywhere else.

pub mod numbering;
pub mod plate_list;

use leptos::prelude::*;
use presenter_core::{
    ActiveNameplate, LiveEvent, Nameplate, StageDisplaySnapshot, StreamElementProps,
    StreamOutputDef,
};
use serde::Serialize;

// ---- Client write DTOs (camelCase, mirroring router/stream_nameplates.rs) --

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NameplateTextReq {
    primary_text: String,
    secondary_text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetActiveReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
}

/// Reorder body: the FULL id set of the output's plates in the new order.
#[derive(Serialize)]
struct ReorderReq {
    ids: Vec<i64>,
}

// ---- REST paths ------------------------------------------------------------

/// The output's plate list (GET list, POST add).
pub fn nameplates_path(slug: &str) -> String {
    format!("/stream/api/outputs/{slug}/nameplates")
}

fn nameplates_order_path(slug: &str) -> String {
    format!("/stream/api/outputs/{slug}/nameplates/order")
}

/// The plate on air (GET cold load, PUT show / hide).
pub fn nameplates_active_path(slug: &str) -> String {
    format!("/stream/api/outputs/{slug}/nameplates/active")
}

/// One plate (id-scoped, not output-scoped): PATCH texts, DELETE.
fn nameplate_path(id: i64) -> String {
    format!("/stream/api/nameplates/{id}")
}

// ---- Pure helpers (host-tested) ---------------------------------------------

/// The id list after moving `id` one step up or down, or `None` when it cannot
/// move (already first / last, or not in the list). The server's reorder wants
/// the FULL id set, so the result always holds every id.
pub fn reorder_one_step(ids: &[i64], id: i64, up: bool) -> Option<Vec<i64>> {
    let pos = ids.iter().position(|x| *x == id)?;
    let target = if up {
        pos.checked_sub(1)?
    } else if pos + 1 < ids.len() {
        pos + 1
    } else {
        return None;
    };
    let mut moved = ids.to_vec();
    moved.swap(pos, target);
    Some(moved)
}

/// The song row's live line: "title · library", the title alone without a
/// library, "—" while no song is live.
pub fn song_line(name: &str, library: &str) -> String {
    match (name.is_empty(), library.is_empty()) {
        (true, _) => "—".to_string(),
        (false, true) => name.to_string(),
        (false, false) => format!("{name} · {library}"),
    }
}

/// The song plate's texts from a stage snapshot: the live song title + library
/// (the same pair the server resolves the song plate from, #779).
pub fn song_texts(snapshot: &StageDisplaySnapshot) -> (String, String) {
    (
        snapshot.song_name.clone().unwrap_or_default(),
        snapshot.library_name.clone().unwrap_or_default(),
    )
}

/// True when the output has a `lower_third` element in any scene, i.e. a layer
/// a plate can show on.
pub fn def_has_lower_third(def: &StreamOutputDef) -> bool {
    def.scenes
        .iter()
        .flat_map(|s| &s.elements)
        .any(|e| matches!(e.props, StreamElementProps::LowerThird { .. }))
}

// ---- The store ---------------------------------------------------------------

/// One output's plates, the plate on air and the live song, plus every plate
/// REST call. `Copy` (signals + a callback), so it threads into each row.
#[derive(Clone, Copy)]
pub struct NameplateStore {
    /// The output whose plates are listed: the editor's selected output (its
    /// own signal, so an output switch moves the list too), `stream` in the
    /// operator view.
    pub output_slug: RwSignal<String>,
    /// The person plates in list order (`position`, then id).
    pub plates: RwSignal<Vec<Nameplate>>,
    /// The plate on air (drives the on-air highlight).
    pub active: RwSignal<Option<ActiveNameplate>>,
    /// The live song title + library (the "Pieseň" row).
    pub song: RwSignal<(String, String)>,
    /// The page's own toast: `(message, "success" | "error")`.
    toast: Callback<(String, &'static str)>,
}

impl NameplateStore {
    /// An empty store for `output_slug`; call [`Self::load`] to fill it.
    pub fn new(output_slug: RwSignal<String>, toast: Callback<(String, &'static str)>) -> Self {
        Self {
            output_slug,
            plates: RwSignal::new(Vec::new()),
            active: RwSignal::new(None),
            song: RwSignal::new((String::new(), String::new())),
            toast,
        }
    }

    /// Report through the page's toast.
    fn notify(self, message: String, variant: &'static str) {
        self.toast.run((message, variant));
    }

    /// Whether `slug` is still the listed output: a response for an output the
    /// page has left since (an editor output switch) is dropped.
    fn is_current(self, slug: &str) -> bool {
        let current = self.output_slug.get_untracked();
        if current == slug {
            return true;
        }
        leptos::logging::log!("nameplates: dropping a response for {slug:?} (now {current:?})");
        false
    }

    /// Cold load: the plate list, the plate on air and the live song.
    pub fn load(self) {
        self.reload_list();
        self.reload_active();
        self.reload_song();
    }

    /// Re-fetch the plate list (mount, after an own write, on a
    /// `StreamNameplatesChanged`).
    pub fn reload_list(self) {
        let slug = self.output_slug.get_untracked();
        leptos::task::spawn_local(async move {
            let result = crate::api::get_json::<Vec<Nameplate>>(&nameplates_path(&slug)).await;
            if !self.is_current(&slug) {
                return;
            }
            match result {
                Ok(list) => self.plates.set(list),
                Err(e) => self.notify(format!("Načítanie menoviek zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Cold-load the plate on air so the highlight is right from the start.
    pub fn reload_active(self) {
        let slug = self.output_slug.get_untracked();
        leptos::task::spawn_local(async move {
            let path = nameplates_active_path(&slug);
            let result = crate::api::get_json::<Option<ActiveNameplate>>(&path).await;
            if let Ok(active) = result {
                if self.is_current(&slug) {
                    self.active.set(active);
                }
            }
        });
    }

    /// Cold-load the live song for the "Pieseň" row (the stage snapshot's
    /// `song_name` / `library_name` are layout-independent).
    pub fn reload_song(self) {
        leptos::task::spawn_local(async move {
            if let Ok(snapshot) = crate::api::stage::get_snapshot().await {
                self.song.set(song_texts(&snapshot));
            }
        });
    }

    /// Keep the store current from a live event: the plate on air, the plate
    /// list and the live song. Other events are ignored.
    pub fn apply_event(self, event: &LiveEvent) {
        match event {
            LiveEvent::StreamNameplate { output, active }
                if *output == self.output_slug.get_untracked() =>
            {
                self.active.set(active.clone());
            }
            LiveEvent::StreamNameplatesChanged { output }
                if *output == self.output_slug.get_untracked() =>
            {
                self.reload_list();
            }
            LiveEvent::Stage { snapshot } => self.song.set(song_texts(snapshot)),
            _ => {}
        }
    }

    /// Add a person plate, then refetch the list.
    pub fn add(self, primary: String, secondary: String) {
        let Some(req) = self.text_req(primary, secondary) else {
            return;
        };
        let slug = self.output_slug.get_untracked();
        leptos::task::spawn_local(async move {
            match crate::api::post_json::<_, Nameplate>(&nameplates_path(&slug), &req).await {
                Ok(_) => {
                    self.reload_list();
                    self.notify("Menovka pridaná.".to_string(), "success");
                }
                Err(e) => self.notify(format!("Pridanie zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Update a plate's texts, then refetch the list.
    pub fn update(self, id: i64, primary: String, secondary: String) {
        let Some(req) = self.text_req(primary, secondary) else {
            return;
        };
        leptos::task::spawn_local(async move {
            match crate::api::patch_json::<_, Nameplate>(&nameplate_path(id), &req).await {
                Ok(_) => self.reload_list(),
                Err(e) => self.notify(format!("Uloženie zlyhalo: {e}"), "error"),
            }
        });
    }

    /// The trimmed texts of an add / edit, or `None` (with a toast) when the
    /// name is empty.
    fn text_req(self, primary: String, secondary: String) -> Option<NameplateTextReq> {
        let primary_text = primary.trim().to_string();
        if primary_text.is_empty() {
            self.notify("Meno nesmie byť prázdne.".to_string(), "error");
            return None;
        }
        Some(NameplateTextReq {
            primary_text,
            secondary_text: secondary.trim().to_string(),
        })
    }

    /// Delete a plate (native confirm first), then refetch the list.
    pub fn delete(self, id: i64) {
        let confirmed = crate::utils::window::window()
            .confirm_with_message("Zmazať túto menovku?")
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        leptos::task::spawn_local(async move {
            match crate::api::delete(&nameplate_path(id)).await {
                Ok(()) => {
                    self.reload_list();
                    self.notify("Menovka zmazaná.".to_string(), "success");
                }
                Err(e) => self.notify(format!("Zmazanie zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Move a plate one step up or down (a new Stream Deck button, #870).
    pub fn move_plate(self, id: i64, up: bool) {
        let ids: Vec<i64> = self
            .plates
            .with_untracked(|list| list.iter().map(|p| p.id).collect());
        let Some(ids) = reorder_one_step(&ids, id, up) else {
            return;
        };
        let slug = self.output_slug.get_untracked();
        leptos::task::spawn_local(async move {
            let req = ReorderReq { ids };
            match crate::api::put_no_content(&nameplates_order_path(&slug), &req).await {
                Ok(()) => self.reload_list(),
                Err(e) => self.notify(format!("Zmena poradia zlyhala: {e}"), "error"),
            }
        });
    }

    /// Show a person plate on the REAL output.
    pub fn show_person(self, id: i64) {
        self.set_active(SetActiveReq {
            source: Some("person".to_string()),
            id: Some(id),
        });
    }

    /// Show the virtual song plate on the REAL output.
    pub fn show_song(self) {
        self.set_active(SetActiveReq {
            source: Some("song".to_string()),
            id: None,
        });
    }

    /// Hide whatever plate is on air.
    pub fn hide(self) {
        self.set_active(SetActiveReq {
            source: None,
            id: None,
        });
    }

    /// PUT the show-state and apply the returned plate on air.
    fn set_active(self, req: SetActiveReq) {
        let slug = self.output_slug.get_untracked();
        leptos::task::spawn_local(async move {
            let path = nameplates_active_path(&slug);
            match crate::api::put_json::<_, Option<ActiveNameplate>>(&path, &req).await {
                Ok(active) => {
                    if self.is_current(&slug) {
                        self.active.set(active);
                    }
                }
                Err(e) => self.notify(format!("Menovku sa nepodarilo prepnúť: {e}"), "error"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::{SceneKind, StreamSceneDef};

    #[test]
    fn a_plate_moves_one_step_and_keeps_the_full_id_set() {
        assert_eq!(reorder_one_step(&[1, 2, 3], 2, true), Some(vec![2, 1, 3]));
        assert_eq!(reorder_one_step(&[1, 2, 3], 2, false), Some(vec![1, 3, 2]));
        assert_eq!(reorder_one_step(&[1, 2, 3], 3, true), Some(vec![1, 3, 2]));
    }

    #[test]
    fn the_first_cannot_go_up_the_last_cannot_go_down_an_unknown_id_not_at_all() {
        assert_eq!(reorder_one_step(&[1, 2, 3], 1, true), None);
        assert_eq!(reorder_one_step(&[1, 2, 3], 3, false), None);
        assert_eq!(reorder_one_step(&[1, 2, 3], 9, true), None);
        assert_eq!(reorder_one_step(&[], 1, false), None);
    }

    #[test]
    fn the_song_line_shows_title_and_library() {
        assert_eq!(
            song_line("Ako Ťa mám rád", "Kapela XY"),
            "Ako Ťa mám rád · Kapela XY"
        );
        assert_eq!(song_line("Ako Ťa mám rád", ""), "Ako Ťa mám rád");
        assert_eq!(song_line("", "Kapela XY"), "—");
        assert_eq!(song_line("", ""), "—");
    }

    #[test]
    fn the_nameplate_paths_are_output_scoped_and_id_scoped() {
        assert_eq!(
            nameplates_path("stream"),
            "/stream/api/outputs/stream/nameplates"
        );
        assert_eq!(
            nameplates_order_path("timer"),
            "/stream/api/outputs/timer/nameplates/order"
        );
        assert_eq!(
            nameplates_active_path("stream"),
            "/stream/api/outputs/stream/nameplates/active"
        );
        assert_eq!(nameplate_path(7), "/stream/api/nameplates/7");
    }

    fn def_with(kinds: &[&str]) -> StreamOutputDef {
        let elements = kinds
            .iter()
            .enumerate()
            .map(|(i, kind)| presenter_core::StreamElementDef {
                id: i as i64 + 1,
                z_order: i as i32,
                props: crate::components::stream_editor::props_access::default_element_props(kind),
            })
            .collect();
        StreamOutputDef {
            id: 1,
            slug: "stream".to_string(),
            name: "Stream".to_string(),
            default_transition_ms: 300,
            base_transition_ms: None,
            overlay_transition_ms: None,
            active_scene_id: None,
            config_revision: 1,
            scenes: vec![StreamSceneDef {
                id: 1,
                name: "Menovky".to_string(),
                kind: SceneKind::Overlay,
                position: 0,
                is_active: true,
                transition_ms: None,
                elements,
            }],
        }
    }

    #[test]
    fn a_lower_third_in_any_scene_is_a_nameplate_layer() {
        assert!(def_has_lower_third(&def_with(&["color", "lower_third"])));
        assert!(!def_has_lower_third(&def_with(&["color", "image"])));
        assert!(!def_has_lower_third(&def_with(&[])));
    }
}
