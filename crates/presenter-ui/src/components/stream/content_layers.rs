//! Pure layer-list model behind `CrossfadeText` (#716; "fade through empty"
//! #834). Host-tested — no signals, no timers.
//!
//! A content element renders a list of [`ContentLayer`]s stacked in one grid
//! cell: the current text plus any outgoing text still fading out. For the
//! `FadeThrough` transition the incoming text must NOT be mounted while an old
//! one is leaving, so it waits as the PENDING text:
//!
//! 1. [`fade_through_change`] — every visible layer starts leaving; the new
//!    text becomes pending (a newer change replaces it, an empty one clears
//!    it). With nothing on screen at all, the new text is mounted at once.
//! 2. [`fade_through_settle`] — called once the fade-out of a wave ended: its
//!    layers are dropped and, when no layer is left, the pending text is
//!    mounted (it then fades in). Drop + mount happen in ONE update, so the
//!    wrapper never flickers out and the two texts never share the DOM.
//!
//! Only one fade-out wave is ever in flight: a visible (non-leaving) layer
//! exists only once every leaving layer is gone.

/// One rendered copy of the content: the current text, or an outgoing text still
/// fading out. Keyed on `seq` (a monotonic instance id — NOT the text) so the
/// same text re-appearing while a previous copy is still leaving never collides.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ContentLayer {
    pub(super) seq: u64,
    pub(super) text: String,
    /// `true` for an animated layer (fades in on mount / out when leaving);
    /// `false` for a `Cut` layer (instant). Immutable per layer.
    pub(super) fade: bool,
    /// Set when this layer is fading out and scheduled for removal.
    pub(super) leaving: bool,
}

/// A fresh, visible, animated layer.
fn fading_in(seq: u64, text: String) -> ContentLayer {
    ContentLayer {
        seq,
        text,
        fade: true,
        leaving: false,
    }
}

/// Apply a text change under `FadeThrough`. Returns the seqs that started
/// leaving NOW — the caller schedules one [`fade_through_settle`] for them after
/// the fade-out. An empty return means no new wave started (nothing was
/// visible): either the text was mounted at once, or it now waits behind the
/// wave already in flight.
pub(super) fn fade_through_change(
    layers: &mut Vec<ContentLayer>,
    pending: &mut Option<String>,
    cur: &str,
    mut next_seq: impl FnMut() -> u64,
) -> Vec<u64> {
    let mut started = Vec::new();
    for layer in layers.iter_mut().filter(|l| !l.leaving) {
        layer.leaving = true;
        started.push(layer.seq);
    }
    if layers.is_empty() {
        // Nothing on screen (not even a leaving layer): fade the text straight in.
        *pending = None;
        if !cur.is_empty() {
            layers.push(fading_in(next_seq(), cur.to_string()));
        }
    } else {
        // Wait for the fade-out; only the NEWEST text is kept, an empty one
        // (a clear) means nothing comes back.
        *pending = (!cur.is_empty()).then(|| cur.to_string());
    }
    started
}

/// A fade-out wave ended: drop its layers (`seqs`) and, once no layer remains,
/// mount the pending text so it fades in.
pub(super) fn fade_through_settle(
    layers: &mut Vec<ContentLayer>,
    pending: &mut Option<String>,
    seqs: &[u64],
    mut next_seq: impl FnMut() -> u64,
) {
    layers.retain(|l| !seqs.contains(&l.seq));
    if layers.is_empty() {
        if let Some(text) = pending.take() {
            layers.push(fading_in(next_seq(), text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A monotonic seq source starting at 100.
    fn seqs() -> impl FnMut() -> u64 {
        let mut n = 99;
        move || {
            n += 1;
            n
        }
    }

    fn texts(layers: &[ContentLayer]) -> Vec<(&str, bool)> {
        layers
            .iter()
            .map(|l| (l.text.as_str(), l.leaving))
            .collect()
    }

    fn showing(text: &str) -> Vec<ContentLayer> {
        vec![fading_in(1, text.to_string())]
    }

    #[test]
    fn a_change_fades_the_old_text_out_and_holds_the_new_one() {
        let mut layers = showing("A");
        let mut pending = None;
        let started = fade_through_change(&mut layers, &mut pending, "B", seqs());
        assert_eq!(started, vec![1]);
        assert_eq!(texts(&layers), vec![("A", true)], "B is not mounted yet");
        assert_eq!(pending.as_deref(), Some("B"));
    }

    #[test]
    fn settle_swaps_the_leaving_layer_for_the_pending_text_at_once() {
        let mut layers = showing("A");
        let mut pending = None;
        let mut next = seqs();
        let started = fade_through_change(&mut layers, &mut pending, "B", &mut next);
        fade_through_settle(&mut layers, &mut pending, &started, &mut next);
        assert_eq!(texts(&layers), vec![("B", false)]);
        assert!(layers[0].fade, "the new text fades in");
        assert_eq!(layers[0].seq, 100);
        assert_eq!(pending, None);
    }

    #[test]
    fn a_burst_keeps_only_the_newest_pending_text() {
        let mut layers = showing("A");
        let mut pending = None;
        let mut next = seqs();
        let wave = fade_through_change(&mut layers, &mut pending, "B", &mut next);
        // C and D arrive while A is still fading out: no second wave.
        assert!(fade_through_change(&mut layers, &mut pending, "C", &mut next).is_empty());
        assert!(fade_through_change(&mut layers, &mut pending, "D", &mut next).is_empty());
        assert_eq!(texts(&layers), vec![("A", true)]);
        fade_through_settle(&mut layers, &mut pending, &wave, &mut next);
        assert_eq!(
            texts(&layers),
            vec![("D", false)],
            "B and C are never mounted"
        );
    }

    #[test]
    fn an_empty_text_only_fades_out() {
        let mut layers = showing("A");
        let mut pending = None;
        let mut next = seqs();
        let wave = fade_through_change(&mut layers, &mut pending, "", &mut next);
        assert_eq!(pending, None);
        fade_through_settle(&mut layers, &mut pending, &wave, &mut next);
        assert!(layers.is_empty(), "nothing comes back after a clear");
    }

    #[test]
    fn a_clear_then_a_new_text_during_the_fade_out_shows_the_new_text() {
        let mut layers = showing("A");
        let mut pending = None;
        let mut next = seqs();
        let wave = fade_through_change(&mut layers, &mut pending, "", &mut next);
        assert!(fade_through_change(&mut layers, &mut pending, "B", &mut next).is_empty());
        fade_through_settle(&mut layers, &mut pending, &wave, &mut next);
        assert_eq!(texts(&layers), vec![("B", false)]);
    }

    #[test]
    fn with_nothing_on_screen_the_text_fades_straight_in() {
        let mut layers = Vec::new();
        let mut pending = Some("stale".to_string());
        let started = fade_through_change(&mut layers, &mut pending, "A", seqs());
        assert!(started.is_empty());
        assert_eq!(texts(&layers), vec![("A", false)]);
        assert_eq!(pending, None);

        // …and an empty text on an empty screen stays empty.
        let mut layers = Vec::new();
        assert!(fade_through_change(&mut layers, &mut pending, "", seqs()).is_empty());
        assert!(layers.is_empty());
    }

    #[test]
    fn a_change_during_the_fade_in_starts_a_new_wave() {
        let mut layers = showing("A");
        let mut pending = None;
        let mut next = seqs();
        let wave = fade_through_change(&mut layers, &mut pending, "B", &mut next);
        fade_through_settle(&mut layers, &mut pending, &wave, &mut next);
        // B is now visible (possibly still fading in); C sends it out.
        let wave = fade_through_change(&mut layers, &mut pending, "C", &mut next);
        assert_eq!(wave, vec![100]);
        assert_eq!(texts(&layers), vec![("B", true)]);
        fade_through_settle(&mut layers, &mut pending, &wave, &mut next);
        assert_eq!(texts(&layers), vec![("C", false)]);
    }
}
