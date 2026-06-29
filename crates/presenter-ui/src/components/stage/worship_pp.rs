use leptos::prelude::*;

use super::worship_pp_helpers::{
    active_sidebar_index, current_song_from_entries, next_song_from_entries,
    scroll_active_entry_into_view,
};
use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_effect;
use crate::utils::color::group_pill_style;
use crate::utils::text::{break_if_long, clean_song_name};
use crate::ws::stage::StageWsState;

const CURRENT_MAX_FONT: f64 = 800.0;
const NEXT_MAX_FONT: f64 = 500.0;
const CURRENT_GROUP_MAX_FONT: f64 = 200.0;
const NEXT_GROUP_MAX_FONT: f64 = 200.0;
const CURRENT_SONG_MAX_FONT: f64 = 200.0;
const NEXT_SONG_MAX_FONT: f64 = 200.0;
const STAGE_SLIDE_BREAK_THRESHOLD: usize = 26;

#[component]
pub fn WorshipPp(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let current_text_ref = NodeRef::<leptos::html::Div>::new();
    let next_text_ref = NodeRef::<leptos::html::Div>::new();
    let current_group_ref = NodeRef::<leptos::html::Div>::new();
    let next_group_ref = NodeRef::<leptos::html::Div>::new();
    let current_song_ref = NodeRef::<leptos::html::Div>::new();
    let next_song_ref = NodeRef::<leptos::html::Div>::new();

    let current_text = move || {
        let raw = ctx
            .snapshot
            .get()
            .and_then(|s| {
                s.current.map(|slide| {
                    if !slide.stage.is_empty() {
                        slide.stage
                    } else {
                        slide.main
                    }
                })
            })
            .unwrap_or_default();
        break_if_long(raw, STAGE_SLIDE_BREAK_THRESHOLD)
    };

    let next_text = move || {
        let raw = ctx
            .snapshot
            .get()
            .and_then(|s| {
                s.next.map(|slide| {
                    if !slide.stage.is_empty() {
                        slide.stage
                    } else {
                        slide.main
                    }
                })
            })
            .unwrap_or_default();
        break_if_long(raw, STAGE_SLIDE_BREAK_THRESHOLD)
    };

    let current_group = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.current.and_then(|sl| sl.group))
    };
    let next_group = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.next.and_then(|sl| sl.group))
    };

    let current_group_style = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.current.and_then(|sl| sl.group_color))
            .map(|color| group_pill_style(&color))
            .unwrap_or_default()
    };

    let next_group_style = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.next.and_then(|sl| sl.group_color))
            .map(|color| group_pill_style(&color))
            .unwrap_or_default()
    };

    let current_group_text = move || current_group().unwrap_or_default();
    let next_group_text = move || next_group().unwrap_or_default();

    let playlist_entries = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.playlist_entries)
            .unwrap_or_default()
    };

    // #496: the sidebar <For> keys by the unique row INDEX (a repeated song
    // shares name + presentation_id, which collided under the old name key).
    // Iterate plain indices — the row reads its name/active-state reactively
    // from the snapshot by index, so the entries themselves aren't carried. The
    // `view!` macro needs `each` as a named closure, not an inline `move ||`.
    let entry_indices = move || (0..playlist_entries().len()).collect::<Vec<usize>>();

    // worship-pp specific: derive the CURRENT-song badge from the Presenter
    // playlist's active entry, NOT from AbleSet's server-side s.song_name (#461).
    let current_song_text = move || current_song_from_entries(&playlist_entries());

    // worship-pp specific: derive next-song from the Presenter playlist's
    // entry-after-active, NOT from AbleSet's s.next_song_name. If no entry
    // is active, or the active one is last, returns "" (no next song).
    let next_song_text = move || next_song_from_entries(&playlist_entries());

    // Auto-scroll the playlist sidebar so the ACTIVE song stays visible as the
    // service advances past the ~10 rows that fit at 1080p (#461). Tracks the
    // active entry's INDEX (#496: the explicit triggered occurrence from the
    // snapshot, not the first name/id match) so the scroll fires on each advance
    // AND targets the correct row when a set repeats a song. When the active
    // index changes, defers one tick (Timeout 0) so the `--active` class is
    // applied to the DOM before scrolling, then centers the active row.
    {
        let snapshot = ctx.snapshot;
        Effect::new(move |prev: Option<Option<usize>>| {
            let active_idx = snapshot.with(|opt| {
                opt.as_ref().and_then(|s| {
                    s.playlist_entries
                        .as_ref()
                        .and_then(|entries| active_sidebar_index(entries, s.active_entry_index))
                })
            });
            if active_idx.is_some() && active_idx != prev.flatten() {
                gloo_timers::callback::Timeout::new(0, scroll_active_entry_into_view).forget();
            }
            active_idx
        });
    }

    autofit_effect(current_text_ref, CURRENT_MAX_FONT, current_text);
    autofit_effect(next_text_ref, NEXT_MAX_FONT, next_text);
    autofit_effect(
        current_group_ref,
        CURRENT_GROUP_MAX_FONT,
        current_group_text,
    );
    autofit_effect(next_group_ref, NEXT_GROUP_MAX_FONT, next_group_text);
    autofit_effect(current_song_ref, CURRENT_SONG_MAX_FONT, current_song_text);
    autofit_effect(next_song_ref, NEXT_SONG_MAX_FONT, next_song_text);

    view! {
        <div class="stage-container" data-layout="worship-pp">
            <div class="stage-pp__slides-area">
                <div class="stage__current-group">
                    <span class="stage__debug-label">"current-group"</span>
                    <div node_ref=current_group_ref class="stage__group-pill" style=current_group_style>
                        {current_group_text}
                    </div>
                </div>

                <div class="stage__current-song">
                    <span class="stage__debug-label">"current-song"</span>
                    <div node_ref=current_song_ref class="stage__song-name-text">
                        {current_song_text}
                    </div>
                </div>

                <div class="stage__current-slide">
                    <span class="stage__debug-label">"current-slide"</span>
                    <div node_ref=current_text_ref class="stage__slide-text">
                        {current_text}
                    </div>
                </div>

                <div class="stage__next-group">
                    <span class="stage__debug-label">"next-group"</span>
                    <div node_ref=next_group_ref class="stage__group-pill" style=next_group_style>
                        {next_group_text}
                    </div>
                </div>

                <div class="stage__next-song">
                    <span class="stage__debug-label">"next-song"</span>
                    <div node_ref=next_song_ref class="stage__song-name-text">
                        {next_song_text}
                    </div>
                </div>

                <div class="stage__next-slide">
                    <span class="stage__debug-label">"next-slide"</span>
                    <div node_ref=next_text_ref class="stage__slide-text">
                        {next_text}
                    </div>
                </div>
            </div>

            <div class="stage-pp__playlist-sidebar">
                <span class="stage__debug-label">"playlist-sidebar"</span>
                <For
                    each=entry_indices
                    key=|idx| *idx
                    children=move |idx| {
                        // #496: key by INDEX (unique) — a repeated song shares
                        // name AND presentation_id, so name-keying collided and
                        // highlighted/scrolled the wrong occurrence. Both the
                        // active-class AND the display name are read REACTIVELY
                        // from the snapshot by index, so the highlight follows
                        // the triggered occurrence and the name stays correct
                        // even if the playlist is edited live.
                        let snapshot = ctx.snapshot;
                        let is_active = move || {
                            snapshot.with(|opt| {
                                opt.as_ref()
                                    .and_then(|s| {
                                        s.playlist_entries.as_ref().map(|entries| {
                                            active_sidebar_index(entries, s.active_entry_index)
                                                == Some(idx)
                                        })
                                    })
                                    .unwrap_or(false)
                            })
                        };
                        let class = move || {
                            if is_active() {
                                "stage-pp__playlist-entry stage-pp__playlist-entry--active"
                            } else {
                                "stage-pp__playlist-entry"
                            }
                        };
                        let display_name = move || {
                            snapshot.with(|opt| {
                                opt.as_ref()
                                    .and_then(|s| s.playlist_entries.as_ref())
                                    .and_then(|entries| entries.get(idx))
                                    .map(|e| clean_song_name(&e.name))
                                    .unwrap_or_default()
                            })
                        };
                        view! { <div class=class>{display_name}</div> }
                    }
                />
            </div>

            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}
