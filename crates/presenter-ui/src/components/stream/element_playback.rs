//! Playback element for the stream output page (#843): a Google Drive folder's
//! photos and videos played in a loop inside the element's `Frame`.
//!
//! The server mirrors the folder (`GET /stream/api/playback?url=<link>` lists
//! the cached items in Drive's order); this component plays that list and
//! refetches it every 60 s (every 5 s while it is empty), so a folder change
//! reaches the output without a reload. A photo stays `image_seconds`; a video
//! plays to its `ended` (muted unless the operator turned sound on — a browser
//! that refuses autoplay with sound gets it muted, OBS plays it with sound).
//! A video that errors or will not start is skipped after a second; one that
//! stalls (no `ended`, no `error`) is skipped by a watchdog of its duration
//! plus a margin. A folder with ONE video loops it natively (`loop`), so it
//! never remounts. Items change with the element's content transition (cut /
//! crossfade over an opaque base / fade through empty, the fade clamped to
//! half a photo) using the same `@starting-style` + `--leaving` layers as the
//! text crossfade (#716/#834). An empty playlist (no link yet, nothing synced,
//! an empty folder) renders an empty, transparent frame.
//!
//! While the page is HIDDEN the loop holds (#868): an item that finishes then
//! is parked. Once the page is visible again a parked photo stays a fresh
//! full `image_seconds` (the moment a view first shows the output, OBS
//! preview or program, must not land on an item change) and a parked video
//! moves on at once. OBS hides
//! a browser source that no view shows (`WasHidden`, also with `shutdown`
//! off), so an unseen `pre` page used to mount a new photo or video every few
//! seconds for hours. A removed video also drops its media resource at once
//! instead of keeping its player and buffers until a major GC.
//!
//! Timers are `gloo` timeouts that are never cancelled; every callback goes
//! through `try_` accessors and a cursor ticket, so a timer that fires after
//! the element re-rendered (draft preview, def refetch) does nothing.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsValue;
use leptos::web_sys::{HtmlVideoElement, VisibilityState};
use presenter_core::{ContentTransition, Frame, ImageFit, PlaybackItem, PlaybackItemKind};
use wasm_bindgen_futures::JsFuture;

use super::playback_model::{
    clear, hold_ms, refetch_delay_ms, replay_of, settle, show, video_watchdog_ms, Change, Cursor,
    MediaLayer, Replay, FADE_REMOVE_BUFFER_MS,
};
use crate::api;

/// A video that errors (or will not start) is skipped after this long — a
/// pause instead of a tight loop when every item of a folder is broken.
const VIDEO_SKIP_MS: u32 = 1_000;
/// A video whose metadata has not loaded within this long is skipped.
const VIDEO_METADATA_TIMEOUT_MS: u32 = 30_000;

/// The element's reactive state, `Copy` so every timer and handler can own it.
#[derive(Clone, Copy)]
struct Player {
    items: RwSignal<Vec<PlaybackItem>>,
    layers: RwSignal<Vec<MediaLayer>>,
    cursor: StoredValue<Cursor>,
    next_seq: StoredValue<u64>,
    /// The ticket of an item that finished while the page was hidden (#868);
    /// [`watch_visibility`] moves on from it once the page is visible again.
    due: StoredValue<Option<u64>>,
    change: Change,
    image_seconds: u32,
}

impl Player {
    fn bump_seq(self) -> Option<u64> {
        self.next_seq.try_update_value(|seq| {
            *seq += 1;
            *seq
        })
    }

    /// `true` while the element still exists (its owner was not disposed).
    fn alive(self) -> bool {
        self.next_seq.try_get_value().is_some()
    }

    fn is_current(self, ticket: u64) -> bool {
        self.cursor
            .try_with_value(|c| c.is_current(ticket))
            .unwrap_or(false)
    }

    /// One item only: a video then loops natively and never "ends".
    fn single_item(self) -> bool {
        self.items
            .try_with_untracked(|items| items.len() == 1)
            .unwrap_or(false)
    }
}

/// CSS `object-fit` for an `ImageFit`.
fn object_fit(fit: ImageFit) -> &'static str {
    match fit {
        ImageFit::Contain => "contain",
        ImageFit::Cover => "cover",
        ImageFit::Stretch => "fill",
    }
}

#[component]
pub fn ElementPlayback(
    /// `stream_elements.id` — for E2E targeting + a stable DOM identity.
    id: i64,
    /// The pasted Drive folder link (empty = nothing to play yet).
    source_url: String,
    fit: ImageFit,
    frame: Frame,
    opacity: f32,
    image_seconds: u32,
    muted: bool,
    content_transition: ContentTransition,
    /// `z_order`, mirrored to `z-index`.
    z: i32,
) -> impl IntoView {
    let player = Player {
        items: RwSignal::new(Vec::new()),
        layers: RwSignal::new(Vec::new()),
        cursor: StoredValue::new(Cursor::default()),
        next_seq: StoredValue::new(0),
        due: StoredValue::new(None),
        change: Change::of(&content_transition).clamped(image_seconds),
        image_seconds,
    };
    watch_visibility(player);
    // Only a link that names a Drive folder is fetched (the server would 422
    // anything else, and that would log a failed request on the broadcast page).
    if presenter_core::parse_drive_folder_url(&source_url).is_ok() {
        fetch_playlist(player, source_url);
    }

    let container_style = format!(
        "left:{}%;top:{}%;width:{}%;height:{}%;opacity:{};z-index:{};",
        frame.x_pct, frame.y_pct, frame.w_pct, frame.h_pct, opacity, z
    );
    let media_style = format!("object-fit:{};", object_fit(fit));
    let fade_style = format!("transition-duration:{}ms;", player.change.fade_ms());
    let item_count = move || player.items.with(|items| items.len()).to_string();
    let layers_each = move || player.layers.get();

    view! {
        <div
            class="stream-element stream-element--playback"
            data-role="stream-element-playback"
            data-element-id=id.to_string()
            data-item-count=item_count
            style=container_style
        >
            <For
                each=layers_each
                key=|layer| layer.seq
                children=move |layer: MediaLayer| {
                    view! {
                        <PlaybackLayer
                            player=player
                            layer=layer
                            muted=muted
                            media_style=media_style.clone()
                            fade_style=fade_style.clone()
                        />
                    }
                }
            />
        </div>
    }
}

/// One media layer: a photo or a video, fading per the element's transition.
#[component]
fn PlaybackLayer(
    player: Player,
    layer: MediaLayer,
    muted: bool,
    media_style: String,
    fade_style: String,
) -> impl IntoView {
    let seq = layer.seq;
    let ticket = layer.ticket;
    let fade = layer.fade;
    // A keyed `<For>` never re-runs this child when only `leaving` flips, so
    // read it REACTIVELY by seq (#496/#693/#716).
    let class = move || {
        // A crossfade's old layer is `held`: it stays opaque until dropped.
        let fading_out = player
            .layers
            .with(|ls| ls.iter().find(|l| l.seq == seq).map(MediaLayer::fading_out))
            .unwrap_or(true);
        let mut class = String::from("stream-playback__layer");
        if fade {
            class.push_str(" stream-playback__layer--fade");
        }
        if fading_out {
            class.push_str(" stream-playback__layer--leaving");
        }
        class
    };
    let layer_style = if fade { fade_style } else { String::new() };
    let PlaybackItem {
        file_id, kind, url, ..
    } = layer.item;
    let kind_attr = match kind {
        PlaybackItemKind::Image => "image",
        PlaybackItemKind::Video => "video",
    };
    let media = match kind {
        PlaybackItemKind::Image => view! {
            <img
                class="stream-playback__media"
                data-role="stream-playback-media"
                data-file-id=file_id.clone()
                src=url
                alt=""
                style=media_style
                on:error=move |_| skip_later(player, ticket)
            />
        }
        .into_any(),
        PlaybackItemKind::Video => view! {
            <PlaybackVideo
                player=player
                ticket=ticket
                file_id=file_id.clone()
                url=url
                muted=muted
                media_style=media_style
            />
        }
        .into_any(),
    };
    view! {
        <div
            class=class
            data-role="stream-playback-layer"
            data-file-id=file_id
            data-kind=kind_attr
            style=layer_style
        >
            {media}
        </div>
    }
}

/// A video item: plays once to `ended`, then the loop moves on. The only item
/// of a folder loops natively instead (`loop`, read reactively, so a folder
/// that gains items lets the video end and the loop move on).
#[component]
fn PlaybackVideo(
    player: Player,
    ticket: u64,
    file_id: String,
    url: String,
    muted: bool,
    media_style: String,
) -> impl IntoView {
    let video_ref = NodeRef::<leptos::html::Video>::new();
    let metadata_seen = StoredValue::new(false);
    // Start playback once the element is in the DOM (the `autoplay` attribute
    // alone is not enough under a strict autoplay policy, and an unmuted video
    // needs the muted fallback), and arm the no-metadata watchdog.
    Effect::new(move |started: Option<bool>| {
        if started == Some(true) {
            return true;
        }
        match video_ref.get() {
            Some(video) => {
                start_video(&video, muted, player, ticket);
                Timeout::new(VIDEO_METADATA_TIMEOUT_MS, move || {
                    if metadata_seen.try_get_value() == Some(false) {
                        leptos::logging::log!(
                            "stream playback: video metadata never loaded — skipping"
                        );
                        item_done(player, ticket);
                    }
                })
                .forget();
                true
            }
            None => false,
        }
    });
    // A stalled video fires neither `ended` nor `error`: skip it after its own
    // duration plus a margin (a natively looping only item is left alone).
    let on_metadata = move |_| {
        let _ = metadata_seen.try_set_value(true);
        let duration = video_ref
            .get_untracked()
            .map(|video| video.duration())
            .unwrap_or(f64::NAN);
        Timeout::new(video_watchdog_ms(duration), move || {
            if !player.single_item() {
                item_done(player, ticket);
            }
        })
        .forget();
    };
    // #868: a removed video lets go of its media player and buffers now, not
    // at the next major GC (a page with a ~2 MB JS heap seldom runs one). With
    // no source the load algorithm ends at NETWORK_EMPTY without an `error`
    // event, and `load()` also cancels any queued media event, so the
    // `on:error` / `on:ended` closures, dropped right after this cleanup,
    // never fire. A photo keeps its `src`: not needed, each URL is one
    // memory-cache entry.
    on_cleanup(move || {
        if let Some(video) = video_ref.try_get_untracked().flatten() {
            release_video(&video);
        }
    });
    let looping = move || player.items.with(|items| items.len() == 1);
    view! {
        <video
            node_ref=video_ref
            class="stream-playback__media"
            data-role="stream-playback-media"
            data-file-id=file_id
            src=url
            style=media_style
            autoplay
            playsinline
            preload="auto"
            prop:muted=muted
            prop:loop=looping
            on:loadedmetadata=on_metadata
            on:ended=move |_| item_done(player, ticket)
            on:error=move |_| skip_later(player, ticket)
        />
    }
}

/// Fetch the playlist, apply it, and schedule the next fetch (which stops
/// once the element is gone).
fn fetch_playlist(player: Player, source_url: String) {
    leptos::task::spawn_local(async move {
        match api::stream::get_playback_playlist(&source_url).await {
            Ok(playlist) => apply_playlist(player, playlist.items),
            Err(err) => leptos::logging::log!("stream playback: playlist fetch failed: {err}"),
        }
        let empty = player
            .items
            .try_with_untracked(|items| items.is_empty())
            .unwrap_or(true);
        Timeout::new(refetch_delay_ms(empty), move || {
            if player.alive() {
                fetch_playlist(player, source_url);
            }
        })
        .forget();
    });
}

/// A fetched playlist: store it; start the loop when idle, clear when empty.
/// A playing item is never interrupted — the next step reads the new list.
fn apply_playlist(player: Player, items: Vec<PlaybackItem>) {
    let empty = items.is_empty();
    let unchanged = player
        .items
        .try_with_untracked(|old| *old == items)
        .unwrap_or(true);
    if unchanged {
        return;
    }
    if player.items.try_set(items).is_some() {
        return; // the element is gone
    }
    let idle = player
        .cursor
        .try_with_value(Cursor::is_idle)
        .unwrap_or(false);
    if idle || empty {
        advance(player);
    }
}

/// Step to the next item (the first when idle) and show it.
fn advance(player: Player) {
    let Some(items) = player.items.try_get_untracked() else {
        return;
    };
    let Some(step) = player.cursor.try_update_value(|c| c.advance(&items)) else {
        return;
    };
    match step {
        Some((item, ticket)) => show_item(player, item, ticket),
        None => {
            if let Some(leaving) = player.layers.try_update(|ls| clear(ls, player.change)) {
                schedule_settle(player, leaving, None);
            }
        }
    }
}

/// Put `item` on screen; a photo schedules its own end. A one-item folder
/// loops onto itself: its photo stays put, its video restarts with a hard cut
/// (a transition onto the same item would only dip the picture).
fn show_item(player: Player, item: PlaybackItem, ticket: u64) {
    let kind = item.kind;
    let replay = player
        .layers
        .try_with_untracked(|ls| replay_of(ls, &item))
        .unwrap_or(Replay::New);
    let change = match replay {
        Replay::New => player.change,
        Replay::KeepImage | Replay::RestartVideo => Change::Cut,
    };
    // A kept photo is on screen already.
    let mut mounted_now = true;
    if replay != Replay::KeepImage {
        let Some(seq) = player.bump_seq() else {
            return;
        };
        let layer = MediaLayer {
            seq,
            item,
            ticket,
            fade: change.fades(),
            leaving: false,
            held: false,
        };
        let pending = layer.clone();
        let Some(shown) = player.layers.try_update(|ls| show(ls, layer, change)) else {
            return;
        };
        mounted_now = shown.mounted;
        let pending = (!shown.mounted).then_some(pending);
        schedule_settle(player, shown.leaving, pending);
    }
    if kind == PlaybackItemKind::Image {
        let hold = hold_ms(player.image_seconds, change, mounted_now);
        Timeout::new(hold, move || item_done(player, ticket)).forget();
    }
}

/// After the fade: drop the `leaving` layers and mount a waiting fade-through
/// item (only if it is still the current one).
fn schedule_settle(player: Player, leaving: Vec<u64>, pending: Option<MediaLayer>) {
    if leaving.is_empty() && pending.is_none() {
        return;
    }
    let wait = player.change.fade_ms() + FADE_REMOVE_BUFFER_MS;
    Timeout::new(wait, move || {
        let pending = pending.filter(|layer| player.is_current(layer.ticket));
        let _ = player.layers.try_update(|ls| settle(ls, &leaving, pending));
    })
    .forget();
}

/// The item with `ticket` finished (photo time up, video ended): next item.
/// While the page is hidden it is parked in `due` instead (#868), and
/// [`watch_visibility`] moves on once the page is visible again.
fn item_done(player: Player, ticket: u64) {
    if !player.is_current(ticket) {
        return;
    }
    if page_hidden() {
        let _ = player.due.try_set_value(Some(ticket));
        return;
    }
    advance(player);
}

/// `true` while the page is hidden (`document.visibilityState`).
fn page_hidden() -> bool {
    leptos::web_sys::window()
        .and_then(|window| window.document())
        .is_some_and(|document| document.visibility_state() == VisibilityState::Hidden)
}

/// Hold the loop while the page is hidden (#868): on `visibilitychange` back
/// to visible, an item that finished meanwhile (`due`) and is still the
/// current one resumes the loop — a photo with a fresh full hold (the page
/// turns visible when a view first shows the output, OBS preview or program;
/// an item change right then would land on that moment), a video at once (it
/// ended or stalled). The event
/// bubbles from the document to the window; the listener goes with the element.
fn watch_visibility(player: Player) {
    let listener = window_event_listener_untyped("visibilitychange", move |_| {
        if page_hidden() {
            leptos::logging::log!("stream playback: page hidden — loop paused");
            return;
        }
        let due = player.due.try_update_value(|due| due.take()).flatten();
        match due.filter(|ticket| player.is_current(*ticket)) {
            Some(ticket) if showing_photo(player) => {
                leptos::logging::log!("stream playback: page visible — photo held, loop resumed");
                let hold = hold_ms(player.image_seconds, Change::Cut, true);
                Timeout::new(hold, move || item_done(player, ticket)).forget();
            }
            Some(_) => {
                leptos::logging::log!("stream playback: page visible — loop resumed");
                advance(player);
            }
            None => leptos::logging::log!("stream playback: page visible"),
        }
    });
    on_cleanup(move || listener.remove());
}

/// `true` when the item on screen (the newest layer not leaving) is a photo.
fn showing_photo(player: Player) -> bool {
    player
        .layers
        .try_with_untracked(|layers| {
            layers
                .iter()
                .rev()
                .find(|layer| !layer.leaving)
                .is_some_and(|layer| layer.item.kind == PlaybackItemKind::Image)
        })
        .unwrap_or(false)
}

/// Drop a removed video's media resource: pause, no source, `load()`.
fn release_video(video: &HtmlVideoElement) {
    let _ = video.pause();
    let _ = video.remove_attribute("src");
    video.load();
}

/// A video that cannot play: move on after a short pause.
fn skip_later(player: Player, ticket: u64) {
    Timeout::new(VIDEO_SKIP_MS, move || item_done(player, ticket)).forget();
}

/// Play a mounted video. An unmuted video the browser refuses to autoplay is
/// retried muted; one that still does not start is skipped (it would never
/// fire `ended`). Logged with `console.log` — an unplayable item is an
/// operational condition of the folder, not a page fault.
fn start_video(video: &HtmlVideoElement, muted: bool, player: Player, ticket: u64) {
    video.set_muted(muted);
    let promise = match video.play() {
        Ok(promise) => promise,
        Err(err) => {
            leptos::logging::log!("stream playback: play() threw: {err:?}");
            skip_later(player, ticket);
            return;
        }
    };
    let video = video.clone();
    leptos::task::spawn_local(async move {
        let Err(err) = JsFuture::from(promise).await else {
            return;
        };
        if !muted && error_name(&err) == "NotAllowedError" {
            leptos::logging::log!("stream playback: autoplay with sound refused — playing muted");
            video.set_muted(true);
            if let Ok(retry) = video.play() {
                if JsFuture::from(retry).await.is_ok() {
                    return;
                }
            }
        }
        leptos::logging::log!("stream playback: video did not start ({err:?}) — skipping");
        skip_later(player, ticket);
    });
}

/// The `name` of a rejected `play()` promise's `DOMException`.
fn error_name(err: &JsValue) -> String {
    js_sys::Reflect::get(err, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string())
        .unwrap_or_default()
}
