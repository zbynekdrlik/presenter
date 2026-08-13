//! Corpus fixture loading — mirrors `scripts/dev/ai-eval/corpus/SCHEMA.md`
//! field-for-field. Every struct here is a direct `serde::Deserialize`
//! target for one `*.case.json` file; nothing here executes anything.

use anyhow::Context;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// One corpus fixture, as loaded from `corpus/<slice>/<id>.case.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Case {
    pub id: String,
    pub slice: String,
    #[serde(rename = "userMessage")]
    pub user_message: String,
    #[serde(default)]
    pub setup: Option<Setup>,
    #[serde(default)]
    pub expected: Expected,
    /// Where this case was loaded from — NOT part of the JSON schema,
    /// filled in by [`load_corpus`] for error messages / trace filenames.
    #[serde(skip, default)]
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Setup {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub seed: Option<Seed>,
    #[serde(default, rename = "priorTurns")]
    pub prior_turns: Vec<PriorTurn>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PriorTurn {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Seed {
    #[serde(default)]
    pub libraries: Vec<SeedLibrary>,
    #[serde(default, rename = "biblePresentations")]
    pub bible_presentations: Vec<SeedBiblePresentation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedLibrary {
    pub name: String,
    #[serde(default)]
    pub presentations: Vec<SeedPresentation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedPresentation {
    pub name: String,
    #[serde(default)]
    pub slides: Vec<SeedSlide>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedSlide {
    pub main: String,
    #[serde(default)]
    pub translation: String,
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedBiblePresentation {
    pub name: String,
    #[serde(default)]
    pub slides: Vec<SeedBibleSlide>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedBibleSlide {
    pub main: String,
    #[serde(default, rename = "mainReference")]
    pub main_reference: String,
}

/// `expected.deleteGate` — SCHEMA.md's `"blocked" | "allowed" | "n/a"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeleteGateExpectation {
    Blocked,
    Allowed,
    #[serde(rename = "n/a")]
    NotApplicable,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Expected {
    #[serde(default, rename = "toolSequence")]
    pub tool_sequence: Vec<String>,
    #[serde(default, rename = "validationErrors")]
    pub validation_errors: Vec<String>,
    #[serde(default, rename = "selfCorrectWithinRetries")]
    pub self_correct_within_retries: Option<u32>,
    #[serde(default, rename = "deleteGate")]
    pub delete_gate: Option<DeleteGateExpectation>,
    #[serde(default, rename = "verbatimVerses")]
    pub verbatim_verses: Vec<VerbatimVerse>,
    #[serde(default, rename = "overriddenVerses")]
    pub overridden_verses: Vec<OverriddenVerse>,
    #[serde(default, rename = "maxIterations")]
    pub max_iterations: Option<u32>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VerbatimVerse {
    #[serde(rename = "ref")]
    pub reference: String,
    pub translation: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverriddenVerse {
    #[serde(rename = "ref")]
    pub reference: String,
    pub translation: String,
    #[serde(rename = "expectedText")]
    pub expected_text: String,
    #[serde(rename = "dbText")]
    pub db_text: String,
}

/// Every recognised corpus slice directory name.
pub const SLICES: &[&str] = &["worship-crud", "bible-authoring", "adversarial"];

/// Load every `*.case.json` fixture under `corpus_dir`, optionally
/// restricted to one slice. Cases are sorted by id for a deterministic
/// run order (stable trace file naming, stable report ordering).
pub fn load_corpus(corpus_dir: &Path, slice_filter: Option<&str>) -> anyhow::Result<Vec<Case>> {
    let mut cases = Vec::new();
    for &slice in SLICES {
        if let Some(filter) = slice_filter {
            if filter != slice {
                continue;
            }
        }
        let slice_dir = corpus_dir.join(slice);
        if !slice_dir.is_dir() {
            continue;
        }
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&slice_dir)
            .with_context(|| format!("reading corpus slice dir {}", slice_dir.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".case.json"))
            })
            .collect();
        entries.sort();

        for path in entries {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("reading corpus case {}", path.display()))?;
            let mut case: Case = serde_json::from_str(&raw)
                .with_context(|| format!("parsing corpus case {}", path.display()))?;
            if case.slice != slice {
                anyhow::bail!(
                    "corpus case {} declares slice '{}' but lives under the '{}' directory",
                    path.display(),
                    case.slice,
                    slice
                );
            }
            case.source_path = path;
            cases.push(case);
        }
    }
    cases.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(cases)
}
