//! Playback element fields (#843): the Google Drive folder link (with an
//! inline hint when it is not a folder link), seconds per photo, sound, fit,
//! opacity — plus the mirror's status line and „Obnoviť".
//!
//! The status comes from `GET /stream/api/playback?url=<link>` for the link in
//! the draft (a `Memo`, so typing in another field never refetches it). It is
//! polled while it can still change on its own (a sync queued or running, or a
//! saved link the worker has not reached yet), backing off from every 2 s to
//! every 15 s without ever giving up, and reloaded on every config revision
//! (a Save starts the worker's sync). „Obnoviť" posts
//! `/stream/api/playback/refresh` with the link — the server syncs it even
//! before the element is saved. The content transition is the shared
//! `TransitionFields` (element_form.rs).

use chrono::{DateTime, Local, Utc};
use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::{
    parse_drive_folder_url, ImageFit, PlaybackPlaylist, StreamElementProps,
    STREAM_PLAYBACK_IMAGE_SECONDS_MAX, STREAM_PLAYBACK_IMAGE_SECONDS_MIN,
};
use serde::Serialize;

use super::playback_status::{link_problem, poll_delay_ms, should_poll, status_line};
use super::StreamEditorCtx;

#[derive(Serialize)]
struct RefreshReq {
    url: String,
}

/// The form's status state, `Copy` for the async callbacks.
#[derive(Clone, Copy)]
struct StatusState {
    playlist: RwSignal<Option<PlaybackPlaylist>>,
    /// Bumped per request; a response for an older one is dropped.
    request: StoredValue<u64>,
}

fn local_hhmm(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
}

#[component]
pub fn PlaybackFields(draft: RwSignal<StreamElementProps>, ctx: StreamEditorCtx) -> impl IntoView {
    let link = Memo::new(move |_| match draft.get() {
        StreamElementProps::Playback { source_url, .. } => source_url,
        _ => String::new(),
    });
    let link_valid = move || parse_drive_folder_url(&link.get()).is_ok();
    let problem = move || link_problem(&link.get());
    let seconds = move || match draft.get() {
        StreamElementProps::Playback { image_seconds, .. } => image_seconds.to_string(),
        _ => String::new(),
    };
    let muted = move || {
        matches!(
            draft.get(),
            StreamElementProps::Playback { muted: true, .. }
        )
    };
    let fit = move || match draft.get() {
        StreamElementProps::Playback { fit, .. } => match fit {
            ImageFit::Contain => "contain",
            ImageFit::Cover => "cover",
            ImageFit::Stretch => "stretch",
        },
        _ => "contain",
    };

    let status = StatusState {
        playlist: RwSignal::new(None),
        request: StoredValue::new(0),
    };
    let refreshing = RwSignal::new(false);
    // The output's config revision: a Save (or any config write) bumps it and
    // starts the worker's sync, so the status is loaded — and polled — again.
    let revision = Memo::new(move |_| {
        ctx.def
            .with(|def| def.as_ref().map(|def| def.config_revision))
    });
    // A new link forgets the old status; a new revision keeps it on screen
    // while it reloads.
    Effect::new(move |previous: Option<String>| {
        let url = link.get();
        revision.track();
        if previous.as_deref() != Some(url.as_str()) {
            status.playlist.set(None);
        }
        if parse_drive_folder_url(&url).is_ok() {
            load_status(status, url.clone(), 0);
        } else {
            // Invalidate any in-flight request of the previous link.
            status.request.update_value(|r| *r += 1);
        }
        url
    });
    let line = move || {
        status
            .playlist
            .with(|p| status_line(p.as_ref(), local_hhmm))
    };
    let on_refresh = move |_| {
        let url = link.get_untracked();
        if parse_drive_folder_url(&url).is_err() || refreshing.get_untracked() {
            return;
        }
        refreshing.set(true);
        leptos::task::spawn_local(async move {
            let body = RefreshReq { url: url.clone() };
            let result = crate::api::post_no_content("/stream/api/playback/refresh", &body).await;
            let _ = refreshing.try_set(false);
            match result {
                Ok(()) => load_status(status, url, 0),
                Err(err) => ctx.show_toast(&format!("Obnovenie zlyhalo: {err}"), "error"),
            }
        });
    };

    view! {
        <div class="stream-editor__kind-fields" data-role="stream-playback-fields">
            <label class="stream-editor__field stream-editor__field--wide">
                <span>"Odkaz na priečinok Google Drive"</span>
                <input
                    type="url"
                    data-role="stream-playback-link"
                    placeholder="https://drive.google.com/drive/folders/…"
                    prop:value=move || link.get()
                    on:input=move |ev| {
                        let value = event_target_value(&ev);
                        draft.update(|p| {
                            if let StreamElementProps::Playback { source_url, .. } = p {
                                *source_url = value;
                            }
                        });
                    }
                />
            </label>
            <Show when=move || problem().is_some()>
                <p class="stream-editor__prop-error" data-role="stream-playback-link-error">
                    {move || problem().unwrap_or_default()}
                </p>
            </Show>
            <p class="stream-editor__hint">
                "Priečinok zdieľaj ako „ktokoľvek s odkazom“. Fotky a videá (MP4, WebM) sa hrajú v poradí ako na Drive, dookola."
            </p>
            <Show when=link_valid>
                <div class="stream-editor__playback-status">
                    <span
                        data-role="stream-playback-status"
                        data-state=move || line().state
                    >
                        {move || line().text}
                    </span>
                    <button
                        type="button"
                        class="stream-editor__btn stream-editor__btn--ghost"
                        data-role="stream-playback-refresh"
                        prop:disabled=move || refreshing.get()
                        on:click=on_refresh
                    >
                        "Obnoviť"
                    </button>
                </div>
            </Show>
            <label class="stream-editor__field">
                <span>"Fotka na (s)"</span>
                <input
                    type="number"
                    min=STREAM_PLAYBACK_IMAGE_SECONDS_MIN.to_string()
                    max=STREAM_PLAYBACK_IMAGE_SECONDS_MAX.to_string()
                    step="1"
                    data-role="stream-playback-seconds"
                    prop:value=seconds
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).trim().parse::<u32>() {
                            let v = v.clamp(
                                STREAM_PLAYBACK_IMAGE_SECONDS_MIN,
                                STREAM_PLAYBACK_IMAGE_SECONDS_MAX,
                            );
                            draft.update(|p| {
                                if let StreamElementProps::Playback { image_seconds, .. } = p {
                                    *image_seconds = v;
                                }
                            });
                        }
                    }
                />
            </label>
            <label class="stream-editor__field stream-editor__field--check">
                <input
                    type="checkbox"
                    data-role="stream-playback-muted"
                    prop:checked=muted
                    on:change=move |ev| {
                        let on = event_target_checked(&ev);
                        draft.update(|p| {
                            if let StreamElementProps::Playback { muted, .. } = p {
                                *muted = on;
                            }
                        });
                    }
                />
                <span>"Videá bez zvuku"</span>
            </label>
            <label class="stream-editor__field">
                <span>"Prispôsobenie"</span>
                <select
                    data-role="stream-playback-fit"
                    prop:value=fit
                    on:change=move |ev| {
                        let f = match event_target_value(&ev).as_str() {
                            "cover" => ImageFit::Cover,
                            "stretch" => ImageFit::Stretch,
                            _ => ImageFit::Contain,
                        };
                        draft.update(|p| {
                            if let StreamElementProps::Playback { fit, .. } = p {
                                *fit = f;
                            }
                        });
                    }
                >
                    <option value="contain">"Celé (contain)"</option>
                    <option value="cover">"Vyplniť (cover)"</option>
                    <option value="stretch">"Roztiahnuť (stretch)"</option>
                </select>
            </label>
            // Opacity as an integer PERCENT (#776) — the wire stays 0..=1.
            <super::percent_input::PercentInput draft=draft role="stream-playback-opacity" />
        </div>
    }
}

/// Fetch the status of `url`; poll again while it can still change on its
/// own. A response for a superseded request (a newer link or fetch) is dropped.
fn load_status(status: StatusState, url: String, polls: u32) {
    let Some(request) = status.request.try_update_value(|r| {
        *r += 1;
        *r
    }) else {
        return;
    };
    leptos::task::spawn_local(async move {
        let result = crate::api::stream::get_playback_playlist(&url).await;
        if status.request.try_get_value() != Some(request) {
            return;
        }
        let playlist = match result {
            Ok(playlist) => playlist,
            Err(err) => {
                leptos::logging::log!("stream editor: playback status failed: {err}");
                return;
            }
        };
        let poll = should_poll(&playlist);
        let _ = status.playlist.try_set(Some(playlist));
        if poll {
            // Stops once the form is gone (the stored request is disposed) or a
            // newer request superseded this chain.
            Timeout::new(poll_delay_ms(polls), move || {
                if status.request.try_get_value() == Some(request) {
                    load_status(status, url, polls.saturating_add(1));
                }
            })
            .forget();
        }
    });
}
