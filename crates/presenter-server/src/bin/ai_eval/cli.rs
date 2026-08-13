//! Hand-rolled CLI argument parsing — zero new dependency. `ai_eval` is a
//! non-default, dev-only binary with a small flag surface, so pulling in a
//! crate like `clap` would be pure overhead; this mirrors `run.sh`'s own
//! manual `while [[ $# -gt 0 ]]` parsing style for consistency within the
//! harness.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Run the real agent loop against `--candidate-url`/`--model` for
    /// every selected corpus case, writing one trace JSON per case.
    Drive,
    /// Run the Layer-1 structural scorer over already-captured traces.
    /// Pure — no `--candidate-url`/`--model` needed, no network, no LLM.
    ScoreL1,
    /// `drive`, then `score-l1`. The default when no subcommand is given —
    /// matches #680's own acceptance-criterion invocation shape
    /// (`ai_eval --candidate-url <url> --model <name>` with no subcommand).
    All,
}

#[derive(Debug, Clone)]
pub struct Args {
    pub mode: Mode,
    pub candidate_url: Option<String>,
    pub candidate_model: Option<String>,
    /// `None` = every slice (SCHEMA.md's `all`).
    pub slice: Option<String>,
    pub corpus_dir: PathBuf,
    pub traces_dir: PathBuf,
    pub report_path: PathBuf,
}

/// `scripts/dev/ai-eval/` relative to THIS crate's own manifest dir,
/// resolved at COMPILE time via `CARGO_MANIFEST_DIR` — so every default
/// path is correct regardless of the working directory `cargo run --bin
/// ai_eval` happens to be invoked from.
fn harness_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("scripts")
        .join("dev")
        .join("ai-eval")
}

pub fn usage() -> String {
    format!(
        "Usage: ai_eval [drive|score-l1|all] [OPTIONS]\n\n\
         Subcommands (default: all):\n\
         \x20 drive     drive the candidate model through the corpus, write traces/*.json\n\
         \x20 score-l1  score already-captured traces (pure — no model/network needed)\n\
         \x20 all       drive, then score-l1\n\n\
         Options:\n\
         \x20 --candidate-url URL   OpenAI-compatible /v1 base URL (required for drive/all)\n\
         \x20 --model NAME          model name to request (required for drive/all)\n\
         \x20 --slice SLICE         all (default) | worship-crud | bible-authoring | adversarial\n\
         \x20 --corpus-dir PATH     default: {}\n\
         \x20 --traces-dir PATH     (alias --out) default: {}\n\
         \x20 --report PATH         default: {}\n\
         \x20 -h, --help            show this help\n",
        harness_root().join("corpus").display(),
        harness_root().join("traces").display(),
        harness_root().join("report").join("results.json").display(),
    )
}

fn take_value(raw: &[String], i: usize, flag: &str) -> anyhow::Result<String> {
    raw.get(i + 1)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{flag} requires a value (see --help)"))
}

/// The subcommand, if `raw[0]` names one, plus how many leading args it
/// consumed (0 or 1) — everything after is flags either way.
fn detect_mode(raw: &[String]) -> (Mode, usize) {
    match raw.first().map(String::as_str) {
        Some("drive") => (Mode::Drive, 1),
        Some("score-l1") => (Mode::ScoreL1, 1),
        Some("all") => (Mode::All, 1),
        _ => (Mode::All, 0), // not a recognised subcommand — everything is a flag
    }
}

/// Raw flag values before defaults/validation are applied. Plain data —
/// exists only to keep [`parse_args`] under this project's function-length
/// cap.
#[derive(Default)]
struct RawFlags {
    candidate_url: Option<String>,
    candidate_model: Option<String>,
    slice: Option<String>,
    corpus_dir: Option<PathBuf>,
    traces_dir: Option<PathBuf>,
    report_path: Option<PathBuf>,
}

fn parse_flags(raw: &[String], mut i: usize) -> anyhow::Result<RawFlags> {
    let mut flags = RawFlags::default();
    while i < raw.len() {
        let flag = raw[i].clone();
        match flag.as_str() {
            "--candidate-url" => flags.candidate_url = Some(take_value(raw, i, &flag)?),
            "--model" => flags.candidate_model = Some(take_value(raw, i, &flag)?),
            "--slice" => flags.slice = Some(take_value(raw, i, &flag)?),
            "--corpus-dir" => flags.corpus_dir = Some(PathBuf::from(take_value(raw, i, &flag)?)),
            "--traces-dir" | "--out" => {
                flags.traces_dir = Some(PathBuf::from(take_value(raw, i, &flag)?));
            }
            "--report" => flags.report_path = Some(PathBuf::from(take_value(raw, i, &flag)?)),
            "-h" | "--help" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}\n\n{}", usage()),
        }
        i += 2;
    }
    Ok(flags)
}

pub fn parse_args(raw: &[String]) -> anyhow::Result<Args> {
    let (mode, consumed) = detect_mode(raw);
    let flags = parse_flags(raw, consumed)?;

    if let Some(s) = &flags.slice {
        if s != "all" && !crate::corpus::SLICES.contains(&s.as_str()) {
            anyhow::bail!(
                "unknown --slice '{s}' (expected: all, worship-crud, bible-authoring, adversarial)"
            );
        }
    }

    if matches!(mode, Mode::Drive | Mode::All) {
        if flags.candidate_url.is_none() {
            anyhow::bail!("{:?} stage requires --candidate-url (see --help)", mode);
        }
        if flags.candidate_model.is_none() {
            anyhow::bail!("{:?} stage requires --model (see --help)", mode);
        }
    }

    let root = harness_root();
    Ok(Args {
        mode,
        candidate_url: flags.candidate_url,
        candidate_model: flags.candidate_model,
        slice: flags.slice.filter(|s| s != "all"),
        corpus_dir: flags.corpus_dir.unwrap_or_else(|| root.join("corpus")),
        traces_dir: flags.traces_dir.unwrap_or_else(|| root.join("traces")),
        report_path: flags
            .report_path
            .unwrap_or_else(|| root.join("report").join("results.json")),
    })
}
