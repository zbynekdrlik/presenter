//! #848: the `#bible-clear` covers a cover fade works on: the mapping's own
//! and the ones a host remembers across deck switches, a small set per layer
//! (third review).
//!
//! A deck switch keeps the old deck's cover PLAYING on its layer while
//! `/composition` lists only the new deck (#841), so later shows and hides
//! also fade remembered covers the mapping no longer lists. Which cover plays
//! on a layer is known only when a mapping is fetched: the operator can
//! trigger another one in Arena after that. On SNV every title sits on layer
//! 12, so "one cover per layer, replaced only by one seen playing" degenerated
//! into "remember only playing covers", and a title triggered after its deck's
//! fetch stayed over the verse after a switch back. So a layer keeps up to
//! [`REMEMBERED_PER_LAYER`] covers:
//!
//! - a cover the fetch saw PLAYING collapses its layer to itself: Resolume
//!   plays one clip per layer, so the others are no longer visible. A fetch's
//!   playing covers are remembered before its others ([`mapped_covers`] lists
//!   them first), so the result never depends on the column order (#848
//!   fourth review: a title left of the playing one was lost);
//! - one that was not playing is added (each parameter once, the last seen
//!   last); at the cap the oldest that is not playing goes;
//! - a cover is kept on ONE layer: remembering it removes its parameter from
//!   every other layer (a layer inserted or deleted in Arena shifts every
//!   index).
//!
//! A remembered cover costs what a mapped one does: every clear shows it and
//! every verse hides it, one GET at the first step, then one PUT per 50 ms
//! step over its layer's transition (~46 PUTs over SNV's 2.3 s, each on a
//! fresh connection, #808). Four titles plus a mapped one on another layer
//! are 5 PUTs per step, below the #841 Bible fade's 8. Fading a cover that
//! does not play is invisible. A title that drops out of the memory (a
//! collapse or the cap) keeps the level the last fade left it at in Arena (0
//! after a verse) until the next Bible push on its deck. The memory is reset
//! with the host config, like `TrackedOpacities`.

use super::bible_clear::bible_lane_clips;
use super::bible_clear_cover::CoverClip;
use super::clip_map::ClipMapping;
use std::collections::{HashMap, HashSet};

/// How many covers one layer remembers.
pub(super) const REMEMBERED_PER_LAYER: usize = 4;

/// The remembered covers per layer (1-based, as in
/// `/composition/layers/{n}`), the oldest first.
pub(super) type RememberedCovers = HashMap<usize, Vec<CoverClip>>;

/// Remember `cover` as a fetched mapping lists it (see the module doc).
pub(super) fn remember_cover(remembered: &mut RememberedCovers, cover: CoverClip) {
    forget_remembered_cover(remembered, cover.param_id);
    let layer = remembered.entry(cover.layer).or_default();
    if cover.connected {
        layer.clear();
    } else if layer.len() >= REMEMBERED_PER_LAYER {
        let oldest = layer.iter().position(|kept| !kept.connected).unwrap_or(0);
        layer.remove(oldest);
    }
    layer.push(cover);
}

/// Stop remembering the cover with opacity parameter `param_id`, on every
/// layer.
pub(super) fn forget_remembered_cover(remembered: &mut RememberedCovers, param_id: i64) {
    remembered.retain(|_, layer| {
        layer.retain(|kept| kept.param_id != param_id);
        !layer.is_empty()
    });
}

/// The opacity parameter ids of the mapping's Bible lane clips.
fn lane_params(mapping: &ClipMapping) -> HashSet<i64> {
    bible_lane_clips(mapping)
        .filter_map(|target| target.opacity)
        .map(|opacity| opacity.param_id)
        .collect()
}

/// The `#bible-clear` clips of `mapping` with an opacity parameter, each
/// once, none that is also a Bible lane clip (the lane owns its opacity).
/// The playing ones come first, the others in composition order (layer,
/// then column), so remembering them in this order ([`remember_cover`])
/// never depends on the columns.
pub(super) fn mapped_covers(mapping: &ClipMapping) -> Vec<CoverClip> {
    let mut seen = lane_params(mapping);
    let mut covers: Vec<CoverClip> = mapping
        .bible_clear
        .iter()
        .filter_map(|target| {
            let opacity = target.opacity?;
            seen.insert(opacity.param_id).then_some(CoverClip {
                clip_id: target.clip_id,
                param_id: opacity.param_id,
                mapped_base: opacity.base,
                fade: target.fade,
                layer: target.layer,
                connected: target.connected,
                mapped: true,
            })
        })
        .collect();
    // A stable sort: the playing covers first, the rest keep their order.
    covers.sort_by_key(|cover| !cover.connected);
    covers
}

/// The covers a fade scheduled with `mapping` works on: the mapping's own
/// ([`mapped_covers`]), then each remembered cover the mapping no longer
/// lists (`mapped: false`), each once, none that is a Bible lane clip of the
/// mapping.
pub(super) fn cover_candidates(
    mapping: &ClipMapping,
    remembered: &RememberedCovers,
) -> Vec<CoverClip> {
    let mut covers = mapped_covers(mapping);
    let mut listed = lane_params(mapping);
    listed.extend(covers.iter().map(|cover| cover.param_id));
    let mut gone: Vec<CoverClip> = remembered
        .values()
        .flatten()
        .filter(|cover| listed.insert(cover.param_id))
        .map(|cover| CoverClip {
            mapped: false,
            ..*cover
        })
        .collect();
    gone.sort_unstable_by_key(|cover| cover.param_id);
    covers.extend(gone);
    covers
}
