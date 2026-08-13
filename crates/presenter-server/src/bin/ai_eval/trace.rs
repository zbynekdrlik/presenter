//! Trace format — the driver's captured record of one case's run, and the
//! Layer-1 scorer's ONLY input.
//!
//! `conversation` is literally the production `presenter_server::ai::ChatMessage`
//! vector — the SAME struct `run_agent`/`router/ai.rs` read and write —
//! never a parallel trace schema. Every tool call (name + raw JSON
//! arguments) and every tool result (including the real delete-gate's
//! `{"error":"delete_blocked",...}` marker) already round-trips through it
//! with zero lossy translation, so a trace captured today scores identically
//! whether it came from a live `drive` run or was hand-committed under
//! `golden/`.

use anyhow::Context;
use presenter_server::ai::ChatMessage;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub case_id: String,
    pub slice: String,
    pub candidate_url: String,
    pub candidate_model: String,
    /// The Bible slide character limit in effect during this run (read from
    /// `AppState::get_bible_preferences` at drive time, the SAME call
    /// `run_agent`'s own system-prompt builder makes) — the Layer-1 scorer
    /// needs this to replay `compose_bible_items_into_slides`/
    /// `validate_bible_slide` with the exact bound production used.
    pub char_limit: u32,
    /// Number of `ChatMessage` entries that existed in `conversation`
    /// BEFORE this case's `userMessage` turn started (from
    /// `setup.priorTurns`). The scorer analyzes only
    /// `conversation[prior_turn_count..]` — this turn's own activity —
    /// never the seeded prior-turn history.
    pub prior_turn_count: usize,
    /// The full conversation after `run_agent` returned (or as far as it
    /// got before an error) — every tool call, every tool result, the
    /// final assistant text, all in the exact wire shape production uses.
    pub conversation: Vec<ChatMessage>,
    pub final_response: Option<String>,
    /// Set when `run_agent` itself returned `Err` (network failure,
    /// provider error, context-budget refusal, ...). The case is still
    /// recorded and scored as a hard failure rather than silently dropped
    /// — one bad case must never abort the whole corpus run.
    pub error: Option<String>,
    /// RFC 3339 timestamp of when this trace was captured.
    pub captured_at: String,
}

impl Trace {
    pub fn write_to(&self, dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating trace output dir {}", dir.display()))?;
        let path = dir.join(format!("{}.json", self.case_id));
        let json = serde_json::to_string_pretty(self)
            .with_context(|| format!("serializing trace for case {}", self.case_id))?;
        std::fs::write(&path, json).with_context(|| format!("writing trace {}", path.display()))?;
        Ok(())
    }

    pub fn read_from(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading trace {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parsing trace {}", path.display()))
    }
}

/// Read every `<caseId>.json` trace file directly under `dir` (non-recursive
/// — traces are flat, one file per case, unlike the corpus which is nested
/// per slice).
pub fn load_traces(dir: &Path) -> anyhow::Result<Vec<Trace>> {
    let mut traces = Vec::new();
    if !dir.is_dir() {
        return Ok(traces);
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading trace dir {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort();
    for path in entries {
        traces.push(Trace::read_from(&path)?);
    }
    Ok(traces)
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
