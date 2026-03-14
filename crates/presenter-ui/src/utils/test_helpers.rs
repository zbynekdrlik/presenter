use crate::api::presentations::SlideInput;
use crate::state::operator::OperatorState;
use crate::state::AppContext;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;

pub fn expose_globals(ctx: &AppContext, op: &OperatorState) {
    let window = web_sys::window().expect("no global window");

    // __presenterLiveConnected
    let connected = ctx.ws_connected;
    let getter =
        Closure::wrap(
            Box::new(move || -> JsValue { JsValue::from_bool(connected.get_untracked()) })
                as Box<dyn Fn() -> JsValue>,
        );
    let _ = js_sys::Reflect::set(
        &window,
        &JsValue::from_str("__presenterLiveConnected"),
        getter.as_ref(),
    );
    getter.forget();

    // __presenterTimers
    let timers_sig = ctx.timers;
    let timers_getter = Closure::wrap(Box::new(move || -> JsValue {
        match timers_sig.get_untracked() {
            Some(t) => serde_wasm_bindgen::to_value(&t).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }) as Box<dyn Fn() -> JsValue>);
    let _ = js_sys::Reflect::set(
        &window,
        &JsValue::from_str("__presenterTimers"),
        timers_getter.as_ref(),
    );
    timers_getter.forget();

    // __presenterStageLayout
    let layout_sig = ctx.stage_layout_code;
    let layout_getter = Closure::wrap(Box::new(move || -> JsValue {
        JsValue::from_str(&layout_sig.get_untracked())
    }) as Box<dyn Fn() -> JsValue>);
    let _ = js_sys::Reflect::set(
        &window,
        &JsValue::from_str("__presenterStageLayout"),
        layout_getter.as_ref(),
    );
    layout_getter.forget();

    // __presenterOperatorState
    let view_sig = ctx.view;
    let mode_sig = ctx.mode;
    let lib_sig = ctx.selected_library_id;
    let plist_sig = ctx.selected_playlist_id;
    let pres_sig = ctx.selected_presentation_id;
    let search_sig = op.search_query;
    let modal_sig = op.open_modal;
    let state_getter = Closure::wrap(Box::new(move || -> JsValue {
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &obj,
            &"view".into(),
            &JsValue::from_str(&view_sig.get_untracked()),
        );
        let _ = js_sys::Reflect::set(
            &obj,
            &"mode".into(),
            &JsValue::from_str(&mode_sig.get_untracked()),
        );
        let _ = js_sys::Reflect::set(
            &obj,
            &"activeLibraryId".into(),
            &match lib_sig.get_untracked() {
                Some(id) => JsValue::from_str(&id),
                None => JsValue::NULL,
            },
        );
        let _ = js_sys::Reflect::set(
            &obj,
            &"activePlaylistId".into(),
            &match plist_sig.get_untracked() {
                Some(id) => JsValue::from_str(&id),
                None => JsValue::NULL,
            },
        );
        let _ = js_sys::Reflect::set(
            &obj,
            &"currentPresentationId".into(),
            &match pres_sig.get_untracked() {
                Some(id) => JsValue::from_str(&id),
                None => JsValue::NULL,
            },
        );
        let _ = js_sys::Reflect::set(
            &obj,
            &"searchQuery".into(),
            &JsValue::from_str(&search_sig.get_untracked()),
        );
        let _ = js_sys::Reflect::set(
            &obj,
            &"modalOpen".into(),
            &match modal_sig.get_untracked() {
                Some(m) => JsValue::from_str(&m),
                None => JsValue::NULL,
            },
        );
        obj.into()
    }) as Box<dyn Fn() -> JsValue>);
    let _ = js_sys::Reflect::set(
        &window,
        &JsValue::from_str("__presenterOperatorState"),
        state_getter.as_ref(),
    );
    state_getter.forget();

    // __presenterOperatorTestHelpers
    create_test_helpers(ctx, op);
}

fn create_test_helpers(ctx: &AppContext, op: &OperatorState) {
    let window = web_sys::window().expect("no global window");
    let helpers = js_sys::Object::new();

    // addPresentationToPlaylist(playlistId, presentationId)
    {
        let playlists = ctx.playlists;
        let func = Closure::wrap(Box::new(
            move |playlist_id: String, presentation_id: String| -> js_sys::Promise {
                let playlists = playlists;
                wasm_bindgen_futures::future_to_promise(async move {
                    let playlist = playlists
                        .get_untracked()
                        .into_iter()
                        .find(|p| p.id.to_string() == playlist_id);
                    if let Some(pl) = playlist {
                        let entries: Vec<crate::api::playlists::PlaylistEntryPayload> = pl
                            .entries
                            .iter()
                            .map(|e| match &e.kind {
                                presenter_core::playlist::PlaylistEntryKind::Presentation {
                                    presentation_id,
                                    ..
                                } => crate::api::playlists::PlaylistEntryPayload::Presentation {
                                    entry_id: Some(e.id.to_string()),
                                    presentation_id: presentation_id.to_string(),
                                },
                                presenter_core::playlist::PlaylistEntryKind::Separator { name } => {
                                    crate::api::playlists::PlaylistEntryPayload::Separator {
                                        entry_id: Some(e.id.to_string()),
                                        name: name.clone(),
                                    }
                                }
                            })
                            .collect();
                        match crate::api::playlists::add_presentation_to_playlist(
                            &playlist_id,
                            &presentation_id,
                            &entries,
                        )
                        .await
                        {
                            Ok(updated) => {
                                playlists.update(|list| {
                                    if let Some(p) =
                                        list.iter_mut().find(|p| p.id.to_string() == playlist_id)
                                    {
                                        *p = updated;
                                    }
                                });
                                Ok(JsValue::TRUE)
                            }
                            Err(_) => Ok(JsValue::FALSE),
                        }
                    } else {
                        Ok(JsValue::FALSE)
                    }
                })
            },
        ) as Box<dyn Fn(String, String) -> js_sys::Promise>);
        let _ = js_sys::Reflect::set(&helpers, &"addPresentationToPlaylist".into(), func.as_ref());
        func.forget();
    }

    // playlistPresentationCount(playlistId)
    {
        let playlists = ctx.playlists;
        let func = Closure::wrap(Box::new(move |playlist_id: String| -> JsValue {
            let count = playlists
                .get_untracked()
                .iter()
                .find(|p| p.id.to_string() == playlist_id)
                .map(|p| {
                    p.entries
                        .iter()
                        .filter(|e| {
                            matches!(
                                e.kind,
                                presenter_core::playlist::PlaylistEntryKind::Presentation { .. }
                            )
                        })
                        .count()
                })
                .unwrap_or(0);
            JsValue::from_f64(count as f64)
        }) as Box<dyn Fn(String) -> JsValue>);
        let _ = js_sys::Reflect::set(&helpers, &"playlistPresentationCount".into(), func.as_ref());
        func.forget();
    }

    // reorderSlides(presentationId, slideIds)
    {
        let presentation = ctx.selected_presentation;
        let func = Closure::wrap(Box::new(
            move |pres_id: String, slide_ids_js: JsValue| -> js_sys::Promise {
                let presentation = presentation;
                let slide_ids: Vec<String> =
                    serde_wasm_bindgen::from_value(slide_ids_js).unwrap_or_default();
                wasm_bindgen_futures::future_to_promise(async move {
                    match crate::api::presentations::reorder_slides(&pres_id, slide_ids).await {
                        Ok(new_slides) => {
                            presentation.update(|p| {
                                if let Some(pres) = p.as_mut() {
                                    pres.slides = new_slides;
                                }
                            });
                            Ok(JsValue::TRUE)
                        }
                        Err(_) => Ok(JsValue::FALSE),
                    }
                })
            },
        ) as Box<dyn Fn(String, JsValue) -> js_sys::Promise>);
        let _ = js_sys::Reflect::set(&helpers, &"reorderSlides".into(), func.as_ref());
        func.forget();
    }

    // slideOrder(presentationId)
    {
        let presentation = ctx.selected_presentation;
        let func = Closure::wrap(Box::new(move |_pres_id: String| -> JsValue {
            let ids: Vec<String> = presentation
                .get_untracked()
                .map(|p| p.slides.iter().map(|s| s.id.to_string()).collect())
                .unwrap_or_default();
            serde_wasm_bindgen::to_value(&ids).unwrap_or(JsValue::NULL)
        }) as Box<dyn Fn(String) -> JsValue>);
        let _ = js_sys::Reflect::set(&helpers, &"slideOrder".into(), func.as_ref());
        func.forget();
    }

    // stageMonitorCounts()
    {
        let connections = ctx.stage_connections;
        let baseline = ctx.stage_monitor_baseline;
        let func = Closure::wrap(Box::new(move || -> JsValue {
            let conns = connections.get_untracked();
            let connected = conns
                .iter()
                .filter(|c| c.status == presenter_core::StageClientStatus::Connected)
                .count();
            let issues = conns
                .iter()
                .filter(|c| c.status != presenter_core::StageClientStatus::Connected)
                .count();
            let (base_connected, base_issues) = baseline.get_untracked().unwrap_or((0, 0));
            let obj = js_sys::Object::new();
            let _ = js_sys::Reflect::set(
                &obj,
                &"connected".into(),
                &JsValue::from_f64(connected as f64),
            );
            let _ = js_sys::Reflect::set(&obj, &"issues".into(), &JsValue::from_f64(issues as f64));
            let _ = js_sys::Reflect::set(
                &obj,
                &"baselineConnected".into(),
                &JsValue::from_f64(base_connected as f64),
            );
            let _ = js_sys::Reflect::set(
                &obj,
                &"baselineIssues".into(),
                &JsValue::from_f64(base_issues as f64),
            );
            obj.into()
        }) as Box<dyn Fn() -> JsValue>);
        let _ = js_sys::Reflect::set(&helpers, &"stageMonitorCounts".into(), func.as_ref());
        func.forget();
    }

    // resetStageMonitorBaseline()
    {
        let connections = ctx.stage_connections;
        let baseline = ctx.stage_monitor_baseline;
        let func = Closure::wrap(Box::new(move || -> JsValue {
            let conns = connections.get_untracked();
            let connected = conns
                .iter()
                .filter(|c| c.status == presenter_core::StageClientStatus::Connected)
                .count();
            let issues = conns
                .iter()
                .filter(|c| c.status != presenter_core::StageClientStatus::Connected)
                .count();
            baseline.set(Some((connected, issues)));
            // Persist to localStorage
            crate::state::save_baseline_to_storage(connected, issues);
            JsValue::TRUE
        }) as Box<dyn Fn() -> JsValue>);
        let _ = js_sys::Reflect::set(&helpers, &"resetStageMonitorBaseline".into(), func.as_ref());
        func.forget();
    }

    // clearSearch()
    {
        let search_query = op.search_query;
        let search_open = op.search_open;
        let search_results = ctx.search_results;
        let func = Closure::wrap(Box::new(move || -> JsValue {
            search_query.set(String::new());
            search_open.set(false);
            search_results.set(Vec::new());
            JsValue::TRUE
        }) as Box<dyn Fn() -> JsValue>);
        let _ = js_sys::Reflect::set(&helpers, &"clearSearch".into(), func.as_ref());
        func.forget();
    }

    // parseSongText(text)
    {
        let func = Closure::wrap(Box::new(move |text: String| -> JsValue {
            let slides = parse_song_text(&text);
            serde_wasm_bindgen::to_value(&slides).unwrap_or(JsValue::NULL)
        }) as Box<dyn Fn(String) -> JsValue>);
        let _ = js_sys::Reflect::set(&helpers, &"parseSongText".into(), func.as_ref());
        func.forget();
    }

    let _ = js_sys::Reflect::set(
        &window,
        &JsValue::from_str("__presenterOperatorTestHelpers"),
        &helpers,
    );
}

/// Regex-like pattern matching for group names (Verse, Chorus, etc.)
fn is_group_line(line: &str) -> bool {
    let line_lower = line.trim().to_lowercase();
    let patterns = [
        "verse",
        "chorus",
        "bridge",
        "intro",
        "outro",
        "pre-chorus",
        "prechorus",
        "tag",
        "interlude",
        "refrain",
        "hook",
        "coda",
        "ending",
        "instrumental",
    ];
    for pattern in patterns {
        if let Some(rest) = line_lower.strip_prefix(pattern) {
            // Check if rest is empty or just a number/whitespace
            let rest = rest.trim();
            if rest.is_empty()
                || rest
                    .chars()
                    .all(|c| c.is_ascii_digit() || c.is_whitespace())
            {
                return true;
            }
        }
    }
    false
}

pub fn parse_song_text(text: &str) -> Vec<SlideInput> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .split("\n\n")
        .filter(|section| !section.trim().is_empty())
        .map(|section| {
            let section = section.trim();
            let lines: Vec<&str> = section.lines().collect();

            // Check if first line is a group name
            if let Some(first_line) = lines.first() {
                if is_group_line(first_line) {
                    let group_name = first_line.trim().to_string();
                    let main_content = lines[1..].join("\n");
                    return SlideInput {
                        main: main_content,
                        translation: None,
                        stage: None,
                        group: Some(group_name),
                    };
                }
            }

            SlideInput {
                main: section.to_string(),
                translation: None,
                stage: None,
                group: None,
            }
        })
        .collect()
}
