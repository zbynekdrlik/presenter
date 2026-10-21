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

mod drag;
mod focus;
pub mod numbering;
pub mod operator_view;
pub mod order;
pub mod plate_list;
mod save_queue;

use leptos::prelude::*;
use presenter_core::{
    ActiveNameplate, LiveEvent, Nameplate, StageDisplaySnapshot, StreamElementProps,
    StreamOutputDef,
};
pub use save_queue::SaveOutcome;
use save_queue::SaveQueue;
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

/// The song row's live line: "title · library", "—" while no song is live.
///
/// #871 changed one case on purpose: a song WITHOUT a library shows its title
/// alone. The editor's row before #871 printed "title · " with a dangling
/// separator there.
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

/// The reorder PUTs of one store, sent one at a time (#871 review): two quick
/// moves must reach the server in order, and only the newest order matters.
#[derive(Default)]
struct OrderSync {
    /// A PUT is on its way.
    busy: bool,
    /// The newest order chosen while it was (output slug, full id set).
    next: Option<(String, Vec<i64>)>,
}

/// A plate save waiting in the [`SaveQueue`]: the texts to PATCH and what
/// to run once the save is done (the row settles its drafts).
struct PlateSave {
    req: NameplateTextReq,
    done: Box<dyn FnOnce(SaveOutcome)>,
}

/// A plate save whose PATCH is on its way (started by [`start_save`]).
struct SentSave {
    fetch: Result<js_sys::Promise, crate::api::ApiError>,
    done: Box<dyn FnOnce(SaveOutcome)>,
}

/// Start the PATCH of a plate save: a keepalive fetch, already handed to the
/// browser when this returns, so a page reloaded or closed right after (a
/// save made as the page goes away) still sends it (#871 review).
fn start_save(id: i64, save: PlateSave) -> SentSave {
    let PlateSave { req, done } = save;
    SentSave {
        fetch: crate::api::start_patch_keepalive(&nameplate_path(id), &req),
        done,
    }
}

/// One output's plates, the plate on air and the live song, plus every plate
/// REST call. `Copy` (signals, stored values and a callback), so it threads
/// into each row.
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
    /// Bumped by every list request AND every local change of the list: a
    /// list response is applied only while its number is still the newest, so
    /// an older (or slower) response never replaces a newer state.
    list_gen: StoredValue<u64>,
    /// The serialized reorder PUTs ([`OrderSync`]).
    order_sync: StoredValue<OrderSync>,
    /// The serialized saves, one PATCH per plate on its way ([`SaveQueue`]).
    /// Local storage: a waiting save holds its `done` closure.
    saves: StoredValue<SaveQueue<PlateSave>, LocalStorage>,
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
            list_gen: StoredValue::new(0),
            order_sync: StoredValue::new(OrderSync::default()),
            saves: StoredValue::new_local(SaveQueue::default()),
        }
    }

    /// Report through the page's toast.
    fn notify(self, message: String, variant: &'static str) {
        self.toast.run((message, variant));
    }

    /// The next list generation (see `list_gen`).
    fn next_gen(self) -> u64 {
        self.list_gen
            .try_update_value(|generation| {
                *generation += 1;
                *generation
            })
            .unwrap_or(0)
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

    /// Forget the listed output's plates and plate on air at once (the editor's
    /// output switch), so the previous output's rows never show while the new
    /// output's list loads. An in-flight list response is dropped too.
    pub fn clear(self) {
        self.next_gen();
        self.plates.set(Vec::new());
        self.active.set(None);
    }

    /// Re-fetch the plate list (mount, after an own write, on a
    /// `StreamNameplatesChanged`). Applied only while it is the newest list
    /// request, the output is unchanged and no reorder is still being sent
    /// (that one reloads once it is done). A new order from elsewhere moves
    /// rows, so the list keeps its focus ([`focus::keep_focus`]).
    pub fn reload_list(self) {
        let slug = self.output_slug.get_untracked();
        let generation = self.next_gen();
        leptos::task::spawn_local(async move {
            let result = crate::api::get_json::<Vec<Nameplate>>(&nameplates_path(&slug)).await;
            if !self.is_current(&slug) {
                return;
            }
            if self.list_gen.try_get_value() != Some(generation) {
                leptos::logging::log!("nameplates: dropping an older plate list ({generation})");
                return;
            }
            if self
                .order_sync
                .try_with_value(|sync| sync.busy)
                .unwrap_or(false)
            {
                leptos::logging::log!(
                    "nameplates: a reorder is on its way; keeping the local order"
                );
                return;
            }
            match result {
                Ok(list) => focus::keep_focus(|| self.plates.set(list)),
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

    /// Save a plate's texts. The saved plate goes into the local list at once
    /// (before the refetch), then `done(SaveOutcome::Saved)` runs, so a row
    /// can drop its draft without its old text flashing back. An empty name
    /// is refused here (`Refused`, with a toast); how a sent save can end is
    /// [`SaveOutcome`] (a failed one toasts too).
    ///
    /// One save per plate is on its way at a time ([`SaveQueue`], #871
    /// review): a save made meanwhile waits and goes next (a newer one
    /// replaces it), so the server never applies an older text last. The list
    /// is refetched once nothing of the plate is left to send. `urgent` (the
    /// page is being hidden or closed) sends a save that would wait at once
    /// instead ([`Self::flush_save`]).
    pub fn update(
        self,
        id: i64,
        primary: String,
        secondary: String,
        urgent: bool,
        done: impl FnOnce(SaveOutcome) + 'static,
    ) {
        let Some(req) = self.text_req(primary, secondary) else {
            leptos::logging::log!("nameplates: plate {id} save refused: the name is empty");
            done(SaveOutcome::Refused);
            return;
        };
        let save = PlateSave {
            req,
            done: Box::new(done),
        };
        let Some(first) = self
            .saves
            .try_update_value(|queue| queue.offer(id, save))
            .flatten()
        else {
            if !urgent || !self.flush_save(id) {
                leptos::logging::log!("nameplates: plate {id} save waits for the one on its way");
            }
            return;
        };
        let sent = start_save(id, first);
        leptos::logging::log!("nameplates: plate {id} save sent");
        leptos::task::spawn_local(async move {
            self.finish_save(id, sent).await;
            while let Some(save) = self
                .saves
                .try_update_value(|queue| queue.finish(id))
                .flatten()
            {
                self.finish_save(id, start_save(id, save)).await;
            }
            self.reload_list();
        });
    }

    /// The page is being hidden or closed: send the waiting save of plate
    /// `id` NOW, beside the one on its way, instead of after it (#871
    /// review). A hidden page may be frozen or closed before that one is
    /// done, and the waiting save would never go out. The server keeps the
    /// PATCH it applies last, and this one, sent last, carries the newest
    /// texts. Returns whether a save was waiting.
    pub fn flush_save(self, id: i64) -> bool {
        let Some(save) = self
            .saves
            .try_update_value(|queue| queue.take_waiting(id))
            .flatten()
        else {
            return false;
        };
        let sent = start_save(id, save);
        leptos::logging::log!(
            "nameplates: plate {id} waiting save sent now: the page is being hidden or closed"
        );
        leptos::task::spawn_local(async move {
            self.finish_save(id, sent).await;
            self.reload_list();
        });
        true
    }

    /// Wait for a started save of plate `id` and report it to its row
    /// (`done`). A save that did not go through toasts why
    /// ([`SaveOutcome::message`]).
    async fn finish_save(self, id: i64, sent: SentSave) {
        let SentSave { fetch, done } = sent;
        match crate::api::finish_json::<Nameplate>(fetch).await {
            Ok(saved) => {
                self.next_gen();
                self.plates.update(|list| {
                    if let Some(plate) = list.iter_mut().find(|p| p.id == saved.id) {
                        *plate = saved;
                    }
                });
                done(SaveOutcome::Saved);
            }
            Err(e) => {
                let outcome = SaveOutcome::of_error(&e);
                leptos::logging::log!("nameplates: plate {id} save not done ({outcome:?}): {e}");
                self.notify(outcome.message(&e), "error");
                done(outcome);
            }
        }
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

    /// The plate ids in the order the page shows now (local moves included).
    fn ids_now(self) -> Vec<i64> {
        self.plates
            .with_untracked(|list| list.iter().map(|p| p.id).collect())
    }

    /// Move a plate one step up or down (a new Stream Deck button, #870).
    pub fn move_plate(self, id: i64, up: bool) {
        if let Some(ids) = order::reorder_one_step(&self.ids_now(), id, up) {
            self.reorder(ids);
        }
    }

    /// Drop a dragged plate into `slot` (an [`order::drop_slot`] value).
    pub fn drop_at(self, id: i64, slot: usize) {
        if let Some(ids) = order::reorder_to_slot(&self.ids_now(), id, slot) {
            self.reorder(ids);
        }
    }

    /// Apply a new order: shown at once (the numbers move, and the next move
    /// builds on it), then sent as the full id set. The moved rows keep the
    /// focus (the drag handle under ↑ / ↓, a half-typed field).
    fn reorder(self, ids: Vec<i64>) {
        self.next_gen();
        focus::keep_focus(|| self.plates.update(|list| order::apply_order(list, &ids)));
        self.send_order(self.output_slug.get_untracked(), ids);
    }

    /// PUT an order. While a PUT is on its way the newest order waits and goes
    /// next (older waiting ones are replaced), so the server ends on the order
    /// the operator chose last; the list is reloaded once nothing is pending.
    fn send_order(self, slug: String, ids: Vec<i64>) {
        let queued = self
            .order_sync
            .try_update_value(|sync| {
                if sync.busy {
                    sync.next = Some((slug.clone(), ids.clone()));
                    true
                } else {
                    sync.busy = true;
                    false
                }
            })
            .unwrap_or(true);
        if queued {
            return;
        }
        leptos::task::spawn_local(async move {
            let mut job = Some((slug, ids));
            while let Some((slug, ids)) = job {
                let req = ReorderReq { ids };
                if let Err(e) =
                    crate::api::put_no_content(&nameplates_order_path(&slug), &req).await
                {
                    self.notify(format!("Zmena poradia zlyhala: {e}"), "error");
                }
                job = self
                    .order_sync
                    .try_update_value(|sync| {
                        let next = sync.next.take();
                        sync.busy = next.is_some();
                        next
                    })
                    .flatten();
            }
            self.reload_list();
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
    use presenter_core::{SceneKind, StreamElementDef, StreamSceneDef};

    #[test]
    fn the_song_line_shows_title_and_library() {
        assert_eq!(
            song_line("Ako Ťa mám rád", "Kapela XY"),
            "Ako Ťa mám rád · Kapela XY"
        );
        assert_eq!(song_line("", "Kapela XY"), "—");
        assert_eq!(song_line("", ""), "—");
    }

    #[test]
    fn a_song_without_a_library_shows_its_title_alone_with_no_dangling_separator() {
        assert_eq!(song_line("Ako Ťa mám rád", ""), "Ako Ťa mám rád");
        assert!(!song_line("Ako Ťa mám rád", "").contains('·'));
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

    /// Element props from their wire JSON (the shape the REST API takes).
    fn props(json: serde_json::Value) -> StreamElementProps {
        serde_json::from_value(json).unwrap_or_else(|e| panic!("element props: {e}"))
    }

    fn text_style() -> serde_json::Value {
        serde_json::json!({
            "fontFamily": "Arial", "sizePct": 5.0, "color": "#ffffff",
            "weight": 700, "align": "left", "lineHeight": 1.2
        })
    }

    fn frame() -> serde_json::Value {
        serde_json::json!({ "xPct": 6.0, "yPct": 74.0, "wPct": 46.0, "hPct": 14.0 })
    }

    fn color() -> StreamElementProps {
        props(serde_json::json!({
            "kind": "color", "color": "#000000", "opacity": 1.0, "frame": frame()
        }))
    }

    fn lower_third() -> StreamElementProps {
        props(serde_json::json!({
            "kind": "lower_third", "frame": frame(), "bar_color": "#0f172a",
            "bar_opacity": 0.85, "accent_color": "#38bdf8", "accent_width_pct": 2.5,
            "primary_style": text_style(), "secondary_style": text_style(),
            "padding_pct": 3.0, "animation": "slide_left", "in_ms": 200, "out_ms": 150,
            "auto_hide_s": 0
        }))
    }

    fn def_with(elements: Vec<StreamElementProps>) -> StreamOutputDef {
        let elements = elements
            .into_iter()
            .enumerate()
            .map(|(i, props)| StreamElementDef {
                id: i as i64 + 1,
                z_order: i as i32,
                props,
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
        assert!(def_has_lower_third(&def_with(vec![color(), lower_third()])));
        assert!(!def_has_lower_third(&def_with(vec![color()])));
        assert!(!def_has_lower_third(&def_with(Vec::new())));
    }
}
