//! Pure model behind `ElementPlayback` (#843): where the loop is, and which
//! media layers are on screen. Host-tested — no signals, no timers, no DOM.
//!
//! - [`Cursor`] steps through the playlist and loops. Every step takes a new
//!   TICKET; a photo's timer, a video's `ended`/`error` and a pending
//!   fade-through mount only act while their ticket is still current, so a
//!   late event of an old item can never skip the item on screen.
//! - The layer list mirrors the text crossfade (#716/#834): `Cut` swaps
//!   atomically, `Fade` fades the new item in over the old one — which stays
//!   OPAQUE underneath (`held`) until it is dropped, because two layers fading
//!   against each other dip a full-frame photo to ~75 % mid-fade —, and
//!   `FadeThrough` mounts the new item only after the old one faded out.

use presenter_core::{ContentTransition, PlaybackItem, PlaybackItemKind};

/// Extra wait after a fade before a leaving layer is dropped, so the opacity
/// transition has fully finished (same as the text crossfade).
pub(super) const FADE_REMOVE_BUFFER_MS: u32 = 80;
/// Playlist refetch period while it has items / while it is empty (a fresh
/// link or a first sync shows up within seconds).
const REFETCH_MS: u32 = 60_000;
const REFETCH_EMPTY_MS: u32 = 5_000;
/// Margin after a video's own duration before the watchdog skips it.
const VIDEO_END_MARGIN_MS: f64 = 5_000.0;
/// Watchdog for a video whose duration is unknown (or absurd).
const VIDEO_UNKNOWN_LENGTH_MS: u32 = 10 * 60_000;
/// Longest watchdog a known duration can ask for.
const VIDEO_MAX_WATCHDOG_MS: f64 = 6.0 * 3600.0 * 1000.0;

/// When the output refetches the playlist next.
pub(super) fn refetch_delay_ms(playlist_empty: bool) -> u32 {
    if playlist_empty {
        REFETCH_EMPTY_MS
    } else {
        REFETCH_MS
    }
}

/// How long a video may play before it is skipped even without `ended`
/// (a stalled stream fires neither `ended` nor `error`): its duration plus a
/// margin, or a fixed bound when the duration is unknown / infinite.
pub(super) fn video_watchdog_ms(duration_secs: f64) -> u32 {
    if duration_secs.is_finite() && duration_secs > 0.0 {
        let ms = (duration_secs * 1000.0 + VIDEO_END_MARGIN_MS).min(VIDEO_MAX_WATCHDOG_MS);
        ms as u32
    } else {
        VIDEO_UNKNOWN_LENGTH_MS
    }
}

/// How an item change animates (the element's content transition).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Change {
    Cut,
    Fade(u32),
    FadeThrough(u32),
}

impl Change {
    pub(super) fn of(transition: &ContentTransition) -> Self {
        match transition {
            ContentTransition::Cut => Self::Cut,
            ContentTransition::Fade { duration_ms } => Self::Fade(*duration_ms),
            ContentTransition::FadeThrough { duration_ms } => Self::FadeThrough(*duration_ms),
        }
    }

    pub(super) fn fade_ms(self) -> u32 {
        match self {
            Self::Cut => 0,
            Self::Fade(ms) | Self::FadeThrough(ms) => ms,
        }
    }

    /// Whether layers fade in/out (CSS `--fade`) or swap instantly.
    pub(super) fn fades(self) -> bool {
        self != Self::Cut
    }

    /// The fade, at most half a photo's time — a fade longer than the photo
    /// would start the next change mid-fade and pile layers up.
    pub(super) fn clamped(self, image_seconds: u32) -> Self {
        let max = image_seconds.max(1).saturating_mul(1000) / 2;
        match self {
            Self::Cut => Self::Cut,
            Self::Fade(ms) => Self::Fade(ms.min(max)),
            Self::FadeThrough(ms) => Self::FadeThrough(ms.min(max)),
        }
    }
}

/// How long a photo stays before the next item. When a `FadeThrough` photo
/// had to WAIT for the previous item to fade out (`mounted_now == false`),
/// that wait is added, so the photo is still visible for the full
/// `image_seconds`; a photo that appeared at once gets exactly that.
pub(super) fn hold_ms(image_seconds: u32, change: Change, mounted_now: bool) -> u32 {
    let shown = image_seconds.max(1).saturating_mul(1000);
    match change {
        Change::FadeThrough(ms) if !mounted_now => shown
            .saturating_add(ms)
            .saturating_add(FADE_REMOVE_BUFFER_MS),
        Change::Cut | Change::Fade(_) | Change::FadeThrough(_) => shown,
    }
}

/// The playlist position after `current` (`(index it had, file id)`): the item
/// after it, wrapping to the first. If the current file left the playlist, the
/// item that now sits at its old index comes next. `None` for an empty list.
pub(super) fn next_position(
    current: Option<(usize, &str)>,
    items: &[PlaybackItem],
) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    let next = match current {
        None => 0,
        Some((position, file_id)) => match items.iter().position(|i| i.file_id == file_id) {
            Some(index) => (index + 1) % items.len(),
            None => position % items.len(),
        },
    };
    Some(next)
}

/// Where the loop is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Cursor {
    /// `(playlist index, file id)` of the item on screen; `None` = idle.
    current: Option<(usize, String)>,
    ticket: u64,
}

impl Cursor {
    pub(super) fn is_idle(&self) -> bool {
        self.current.is_none()
    }

    /// `true` while the item that got `ticket` is still the current one.
    pub(super) fn is_current(&self, ticket: u64) -> bool {
        self.current.is_some() && self.ticket == ticket
    }

    /// Step to the next item (the first when idle) and take a new ticket.
    /// `None` — and idle — when the playlist is empty.
    pub(super) fn advance(&mut self, items: &[PlaybackItem]) -> Option<(PlaybackItem, u64)> {
        self.ticket = self.ticket.wrapping_add(1);
        let current = self.current.as_ref().map(|(p, id)| (*p, id.as_str()));
        let item = next_position(current, items).and_then(|next| {
            let item = items.get(next)?.clone();
            Some((next, item))
        });
        match item {
            Some((next, item)) => {
                self.current = Some((next, item.file_id.clone()));
                Some((item, self.ticket))
            }
            None => {
                self.current = None;
                None
            }
        }
    }
}

/// One rendered media item. Keyed on `seq` (never the file id: the same file
/// comes back every loop, possibly while its previous copy still fades out).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MediaLayer {
    pub(super) seq: u64,
    pub(super) item: PlaybackItem,
    /// The cursor ticket this item was shown with.
    pub(super) ticket: u64,
    /// Fades in on mount / out when leaving (`Fade`, `FadeThrough`).
    pub(super) fade: bool,
    /// Scheduled for removal.
    pub(super) leaving: bool,
    /// A leaving layer that stays OPAQUE under the incoming one until it is
    /// dropped (`Fade`) instead of fading out.
    pub(super) held: bool,
}

impl MediaLayer {
    /// Whether the layer is fading out right now (the CSS `--leaving` class).
    pub(super) fn fading_out(&self) -> bool {
        self.leaving && !self.held
    }
}

/// What [`show`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Shown {
    /// Layers that started leaving now — drop them after the fade.
    pub(super) leaving: Vec<u64>,
    /// `false` when the new layer waits for the fade-out (`FadeThrough`); the
    /// caller mounts it with [`settle`].
    pub(super) mounted: bool,
}

/// Put `layer` on screen under `change`.
pub(super) fn show(layers: &mut Vec<MediaLayer>, layer: MediaLayer, change: Change) -> Shown {
    if change == Change::Cut {
        layers.clear();
        layers.push(layer);
        return Shown {
            leaving: Vec::new(),
            mounted: true,
        };
    }
    // A crossfade keeps the old item opaque underneath; a fade-through fades
    // it out first.
    let leaving = start_leaving(layers, matches!(change, Change::Fade(_)));
    let wait = matches!(change, Change::FadeThrough(_)) && !layers.is_empty();
    if !wait {
        layers.push(layer);
    }
    Shown {
        leaving,
        mounted: !wait,
    }
}

/// The playlist became empty: fade everything out (`Cut`: drop it now).
/// Returns the layers that started leaving.
pub(super) fn clear(layers: &mut Vec<MediaLayer>, change: Change) -> Vec<u64> {
    if change == Change::Cut {
        layers.clear();
        return Vec::new();
    }
    start_leaving(layers, false)
}

/// A fade-out ended: drop the `seqs` layers and, when nothing visible is left,
/// mount the `pending` layer (a `FadeThrough` item). Returns whether it was
/// mounted.
pub(super) fn settle(
    layers: &mut Vec<MediaLayer>,
    seqs: &[u64],
    pending: Option<MediaLayer>,
) -> bool {
    layers.retain(|l| !seqs.contains(&l.seq));
    match pending {
        Some(layer) if layers.iter().all(|l| l.leaving) => {
            layers.push(layer);
            true
        }
        _ => false,
    }
}

/// How the next item goes on screen when it may be the item already shown —
/// a one-item folder loops onto itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Replay {
    /// A different item: mount a new layer with the element's transition.
    New,
    /// The same photo is visible: keep it (a crossfade onto itself would dip).
    KeepImage,
    /// The same video ended: mount it again with a hard cut, so it restarts
    /// without fading through itself.
    RestartVideo,
}

pub(super) fn replay_of(layers: &[MediaLayer], item: &PlaybackItem) -> Replay {
    let visible = layers.iter().any(|l| !l.leaving && l.item == *item);
    match (visible, item.kind) {
        (false, _) => Replay::New,
        (true, PlaybackItemKind::Image) => Replay::KeepImage,
        (true, PlaybackItemKind::Video) => Replay::RestartVideo,
    }
}

/// Schedule every visible layer for removal (`held`: keep it opaque until
/// then); returns their seqs.
fn start_leaving(layers: &mut [MediaLayer], held: bool) -> Vec<u64> {
    let mut started = Vec::new();
    for layer in layers.iter_mut().filter(|l| !l.leaving) {
        layer.leaving = true;
        layer.held = held;
        started.push(layer.seq);
    }
    started
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, kind: PlaybackItemKind) -> PlaybackItem {
        PlaybackItem {
            file_id: id.to_string(),
            kind,
            name: format!("{id}.x"),
            url: format!("/stream/playback/F/{id}"),
        }
    }

    fn playlist(ids: &[&str]) -> Vec<PlaybackItem> {
        ids.iter()
            .map(|id| item(id, PlaybackItemKind::Image))
            .collect()
    }

    fn layer(seq: u64, id: &str, change: Change) -> MediaLayer {
        MediaLayer {
            seq,
            item: item(id, PlaybackItemKind::Image),
            ticket: seq,
            fade: change.fades(),
            leaving: false,
            held: false,
        }
    }

    fn visible(layers: &[MediaLayer]) -> Vec<(&str, bool)> {
        layers
            .iter()
            .map(|l| (l.item.file_id.as_str(), l.leaving))
            .collect()
    }

    #[test]
    fn the_cursor_loops_through_the_playlist() {
        let items = playlist(&["a", "b", "c"]);
        let mut cursor = Cursor::default();
        assert!(cursor.is_idle());
        let order: Vec<String> = (0..7)
            .filter_map(|_| cursor.advance(&items).map(|(i, _)| i.file_id))
            .collect();
        assert_eq!(order, ["a", "b", "c", "a", "b", "c", "a"]);
    }

    #[test]
    fn every_step_takes_a_new_ticket_and_old_tickets_go_stale() {
        let items = playlist(&["a", "b"]);
        let mut cursor = Cursor::default();
        let (_, first) = cursor.advance(&items).expect("a");
        assert!(cursor.is_current(first));
        let (_, second) = cursor.advance(&items).expect("b");
        assert!(!cursor.is_current(first), "a's late timer is ignored");
        assert!(cursor.is_current(second));
    }

    #[test]
    fn an_empty_playlist_makes_the_cursor_idle() {
        let mut cursor = Cursor::default();
        let (_, ticket) = cursor.advance(&playlist(&["a"])).expect("a");
        assert!(cursor.advance(&[]).is_none());
        assert!(cursor.is_idle());
        assert!(!cursor.is_current(ticket));
    }

    #[test]
    fn a_changed_playlist_continues_after_the_current_item() {
        // The current item moved: continue after it.
        assert_eq!(
            next_position(Some((0, "a")), &playlist(&["x", "a", "b"])),
            Some(2)
        );
        // The current item left: the item now at its old index comes next.
        assert_eq!(
            next_position(Some((1, "b")), &playlist(&["a", "c", "d"])),
            Some(1)
        );
        assert_eq!(
            next_position(Some((5, "z")), &playlist(&["a", "b"])),
            Some(1)
        );
        assert_eq!(next_position(None, &playlist(&["a"])), Some(0));
        assert_eq!(next_position(Some((0, "a")), &[]), None);
        // A single item loops onto itself.
        assert_eq!(next_position(Some((0, "a")), &playlist(&["a"])), Some(0));
    }

    #[test]
    fn photo_hold_times() {
        assert_eq!(hold_ms(5, Change::Cut, true), 5_000);
        assert_eq!(hold_ms(5, Change::Fade(400), true), 5_000);
        // A fade-through photo that waited for the fade-out gets that time back…
        assert_eq!(hold_ms(5, Change::FadeThrough(400), false), 5_480);
        // …one that appeared at once (first item, after a clear) does not.
        assert_eq!(hold_ms(5, Change::FadeThrough(400), true), 5_000);
        assert_eq!(
            hold_ms(0, Change::Cut, true),
            1_000,
            "never below one second"
        );
    }

    #[test]
    fn a_fade_never_outlasts_half_a_photo() {
        assert_eq!(Change::Fade(3_000).clamped(2), Change::Fade(1_000));
        assert_eq!(
            Change::FadeThrough(800).clamped(5),
            Change::FadeThrough(800)
        );
        assert_eq!(Change::Fade(900).clamped(0), Change::Fade(500));
        assert_eq!(Change::Cut.clamped(1), Change::Cut);
    }

    #[test]
    fn an_empty_playlist_is_refetched_sooner() {
        assert_eq!(refetch_delay_ms(true), 5_000);
        assert_eq!(refetch_delay_ms(false), 60_000);
    }

    #[test]
    fn the_video_watchdog_follows_the_duration() {
        assert_eq!(video_watchdog_ms(12.5), 17_500);
        assert_eq!(video_watchdog_ms(f64::INFINITY), 600_000);
        assert_eq!(video_watchdog_ms(f64::NAN), 600_000);
        assert_eq!(video_watchdog_ms(0.0), 600_000);
        assert_eq!(video_watchdog_ms(1e9), 21_600_000, "capped at 6 h");
    }

    #[test]
    fn transitions_map_from_the_element() {
        assert_eq!(Change::of(&ContentTransition::Cut), Change::Cut);
        assert_eq!(
            Change::of(&ContentTransition::Fade { duration_ms: 300 }),
            Change::Fade(300)
        );
        assert_eq!(
            Change::of(&ContentTransition::FadeThrough { duration_ms: 500 }).fade_ms(),
            500
        );
        assert!(!Change::Cut.fades());
    }

    #[test]
    fn a_cut_swaps_the_layer_at_once() {
        let mut layers = vec![layer(1, "a", Change::Cut)];
        let shown = show(&mut layers, layer(2, "b", Change::Cut), Change::Cut);
        assert_eq!(visible(&layers), [("b", false)]);
        assert!(shown.mounted && shown.leaving.is_empty());
    }

    #[test]
    fn a_fade_keeps_the_old_layer_opaque_under_the_new_one() {
        let change = Change::Fade(300);
        let mut layers = vec![layer(1, "a", change)];
        let shown = show(&mut layers, layer(2, "b", change), change);
        assert_eq!(visible(&layers), [("a", true), ("b", false)]);
        // Held: it does not fade out, the new photo fades in over it.
        assert!(layers[0].held && !layers[0].fading_out());
        assert_eq!(shown.leaving, [1]);
        assert!(shown.mounted);
        assert!(!settle(&mut layers, &shown.leaving, None));
        assert_eq!(visible(&layers), [("b", false)]);
    }

    #[test]
    fn a_fade_through_mounts_the_new_item_only_after_the_fade_out() {
        let change = Change::FadeThrough(300);
        let mut layers = vec![layer(1, "a", change)];
        let next = layer(2, "b", change);
        let shown = show(&mut layers, next.clone(), change);
        assert_eq!(visible(&layers), [("a", true)], "b waits");
        assert!(layers[0].fading_out(), "a fades out first");
        assert!(!shown.mounted);
        assert!(settle(&mut layers, &shown.leaving, Some(next)));
        assert_eq!(visible(&layers), [("b", false)]);
    }

    #[test]
    fn the_first_item_appears_at_once_under_every_transition() {
        for change in [Change::Cut, Change::Fade(300), Change::FadeThrough(300)] {
            let mut layers = Vec::new();
            let shown = show(&mut layers, layer(1, "a", change), change);
            assert!(shown.mounted, "{change:?}");
            assert_eq!(visible(&layers), [("a", false)]);
        }
    }

    #[test]
    fn clearing_fades_everything_out() {
        let mut layers = vec![layer(1, "a", Change::Fade(200))];
        assert_eq!(clear(&mut layers, Change::Fade(200)), [1]);
        assert_eq!(visible(&layers), [("a", true)]);
        assert!(
            layers[0].fading_out(),
            "a clear fades out, nothing replaces it"
        );
        let mut cut = vec![layer(1, "a", Change::Cut)];
        assert!(clear(&mut cut, Change::Cut).is_empty());
        assert!(cut.is_empty());
    }

    #[test]
    fn a_one_item_folder_never_fades_onto_itself() {
        let photo = item("a", PlaybackItemKind::Image);
        let clip = item("v", PlaybackItemKind::Video);
        let showing = |it: &PlaybackItem| {
            vec![MediaLayer {
                seq: 1,
                item: it.clone(),
                ticket: 1,
                fade: true,
                leaving: false,
                held: false,
            }]
        };
        assert_eq!(replay_of(&showing(&photo), &photo), Replay::KeepImage);
        assert_eq!(replay_of(&showing(&clip), &clip), Replay::RestartVideo);
        assert_eq!(replay_of(&showing(&photo), &clip), Replay::New);
        assert_eq!(replay_of(&[], &photo), Replay::New);
        // A copy that is only fading out does not count as shown.
        let mut leaving = showing(&photo);
        leaving[0].leaving = true;
        assert_eq!(replay_of(&leaving, &photo), Replay::New);
    }
}
