//! #870 Companion nameplate GRID: the `stream_nameplate_slot` command.
//!
//! A grid button addresses a SLOT = a 1-based position in the default output's
//! person-plate list, never a plate id. Its label (`nameplate_slot_<n>_*`,
//! `nameplates.rs`) and its press (here) are resolved server-side from the same
//! repository list (`list_stream_nameplates`: position, then id), so they
//! cannot point at different plates. Two guards close the remaining gaps:
//!
//! * Only the default output (`stream`) has slot variables, so a press for any
//!   other output is refused: a button could otherwise show `stream`'s slot N
//!   and fire another output's slot N.
//! * The label reaches Companion through a push (`StreamNameplatesChanged`),
//!   while the press reads the list live. The module sends the plate id its
//!   label shows (`expect_id`); when slot N now holds another plate, or none,
//!   the press is refused ("press again") and nothing goes on air. Without
//!   `expect_id` (an older button, an empty module cache) the slot just fires.

use super::nameplates::DEFAULT_OUTPUT;
use super::stream::{activation_error, resolve_output};
use crate::state::AppState;
use presenter_core::Nameplate;
use serde::Deserialize;
use serde_json::Value;
use tracing::{info, warn};

/// What a press does with the plate in its slot. `toggle` (the default) hides
/// it when it is the one on air, else shows it; `show` always shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum SlotMode {
    #[default]
    Toggle,
    Show,
}

/// A parsed `stream_nameplate_slot` press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SlotPress {
    /// Resolved output slug; only [`DEFAULT_OUTPUT`] is accepted at press time.
    pub(super) output: String,
    /// 1-based position in the plate list (never 0).
    pub(super) slot: usize,
    pub(super) mode: SlotMode,
    /// The plate id the button's label showed (`nameplate_slot_<n>_id`), when
    /// the module had one. `None` = no check.
    pub(super) expect_id: Option<i64>,
}

/// Wire payload `{slot, mode?, expect_id?, output?}`. A missing, negative or
/// non-integer slot fails to deserialize; slot 0 is refused in
/// [`parse_slot_press`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotPayload {
    slot: usize,
    #[serde(default)]
    mode: SlotMode,
    #[serde(default)]
    expect_id: Option<i64>,
    #[serde(default)]
    output: Option<String>,
}

/// Parse a `stream_nameplate_slot` payload (delegated from
/// `stream::parse_stream_command`). Malformed payloads return `Err(message)`,
/// rendered as a non-fatal error reply.
pub(super) fn parse_slot_press(name: &str, payload: Value) -> Result<SlotPress, String> {
    let data: SlotPayload =
        serde_json::from_value(payload).map_err(|err| format!("invalid {name} payload: {err}"))?;
    if data.slot == 0 {
        return Err(format!("{name}: slot must be 1 or higher"));
    }
    Ok(SlotPress {
        output: resolve_output(data.output),
        slot: data.slot,
        mode: data.mode,
        expect_id: data.expect_id,
    })
}

/// Fire a press: resolve its slot to the plate there NOW, then toggle or show
/// that plate. Every refusal is a non-fatal error reply with nothing put on
/// air. Every press logs one line (INFO fired / refused, WARN failed).
pub(super) async fn fire_slot_press(state: &AppState, press: SlotPress) -> Result<(), String> {
    let plate = resolve_slot_plate(state, &press).await?;
    let (output, slot) = (press.output.as_str(), press.slot);
    let outcome = match press.mode {
        SlotMode::Toggle => state.stream_nameplate_toggle_person(output, plate.id).await,
        SlotMode::Show => state.stream_nameplate_show_person(output, plate.id).await,
    };
    match outcome {
        Ok(active) => {
            info!(
                slot,
                plate_id = plate.id,
                name = ?plate.primary_text,
                output,
                mode = ?press.mode,
                on_air = active.is_some(),
                "companion nameplate slot fired"
            );
            Ok(())
        }
        Err(err) => {
            warn!(
                slot,
                plate_id = plate.id,
                name = ?plate.primary_text,
                output,
                error = %err,
                "companion nameplate slot failed"
            );
            Err(activation_error(err))
        }
    }
}

/// Resolve a press to the plate in its slot now. Refused, with one log line
/// each: an output other than [`DEFAULT_OUTPUT`], an unreadable plate list, a
/// slot that no longer holds the plate its label showed (`expect_id`), and an
/// empty slot.
async fn resolve_slot_plate(state: &AppState, press: &SlotPress) -> Result<Nameplate, String> {
    let (output, slot) = (press.output.as_str(), press.slot);
    if output != DEFAULT_OUTPUT {
        info!(
            slot,
            output = ?output,
            "companion nameplate slot refused: not the default output"
        );
        return Err(format!(
            "nameplate slots exist only for output {DEFAULT_OUTPUT:?}, not {output:?}"
        ));
    }
    let plates = match state.repository().list_stream_nameplates(output).await {
        Ok(plates) => plates,
        Err(err) => {
            warn!(
                slot,
                output,
                error = %err,
                "companion nameplate slot refused: plate list unreadable"
            );
            return Err(format!("unknown stream output {output:?}: {err}"));
        }
    };
    let current = slot
        .checked_sub(1)
        .and_then(|index| plates.get(index).cloned());
    match (current, press.expect_id) {
        (Some(plate), Some(expected)) if plate.id != expected => {
            info!(
                slot,
                output,
                expect_id = expected,
                found_id = plate.id,
                found_name = ?plate.primary_text,
                "companion nameplate slot refused: the slot changed since its label"
            );
            Err(slot_changed(slot))
        }
        (None, Some(expected)) => {
            info!(
                slot,
                output,
                expect_id = expected,
                plates = plates.len(),
                "companion nameplate slot refused: the labelled plate left the slot"
            );
            Err(slot_changed(slot))
        }
        (None, None) => {
            info!(
                slot,
                output,
                plates = plates.len(),
                "companion nameplate slot refused: empty slot"
            );
            Err(format!("nameplate slot {slot} is empty in output {output}"))
        }
        (Some(plate), _) => Ok(plate),
    }
}

/// The refusal for a slot whose plate changed between the label and the press.
fn slot_changed(slot: usize) -> String {
    format!("nameplate slot {slot} changed — press again")
}
