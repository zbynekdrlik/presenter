//! Hand-rolled CLI argument parsing — zero new dependency. `ai_eval` is a
//! non-default, dev-only binary with a small flag surface, so pulling in a
//! crate like `clap` would be pure overhead; this mirrors `run.sh`'s own
//! manual `while [[ $# -gt 0 ]]` parsing style for consistency within the
//! harness.
//!
//! `--corpus-dir`/`--traces-dir`/`--report` have NO built-in default (#662
//! defect 3) — earlier versions defaulted them via `env!("CARGO_MANIFEST_DIR")`,
//! a COMPILE-TIME constant baked into the binary at whatever path GitHub's
//! runner happened to check the repo out to. Since this binary is always
//! COPIED OUT of that checkout before it ever runs (dev2 downloads the
//! release artifact to `/tmp/ai-eval-bin/`, nowhere near any repo checkout —
//! see `.claude/rules/ai-eval-harness.md`'s "Running the harness on dev2"),
//! that default was permanently wrong the moment the artifact left the
//! build machine. A required flag with a clear error beats a silently-wrong
//! path.

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
    /// Only meaningful once `score-l1` actually runs (`Mode::ScoreL1` /
    /// `Mode::All`) — `Mode::Drive` alone never reads or writes a report,
    /// so `--report` is not required for it. `None` for a pure `drive` run
    /// that omitted the flag; `parse_args` guarantees `Some` whenever
    /// `score-l1` will run.
    pub report_path: Option<PathBuf>,
    /// `--constrained` (#662 step 2): drive each case through the single-shot
    /// constrained-output path (`response_format` json_schema, no tool loop)
    /// instead of the real `run_agent` tool loop. Only meaningful for the bible
    /// slices; the resulting traces carry `constrained: true` so `score-l1`
    /// skips the (inapplicable) toolSequence check.
    pub constrained: bool,
}

pub fn usage() -> String {
    "Usage: ai_eval [drive|score-l1|all] [OPTIONS]\n\n\
     Subcommands (default: all):\n\
     \x20 drive     drive the candidate model through the corpus, write traces/*.json\n\
     \x20 score-l1  score already-captured traces (pure — no model/network needed)\n\
     \x20 all       drive, then score-l1\n\n\
     Options:\n\
     \x20 --candidate-url URL   OpenAI-compatible /v1 base URL (required for drive/all)\n\
     \x20 --model NAME          model name to request (required for drive/all)\n\
     \x20 --slice SLICE         all (default) | worship-crud | bible-authoring | adversarial\n\
     \x20 --corpus-dir PATH     REQUIRED — path to the corpus/ directory. No built-in default:\n\
     \x20                       this binary is normally run as a standalone artifact copied out\n\
     \x20                       of the repo (see .claude/rules/ai-eval-harness.md's dev2 run\n\
     \x20                       recipe), where a path baked in at compile time would silently\n\
     \x20                       point at the wrong machine's checkout.\n\
     \x20 --traces-dir PATH     (alias --out) REQUIRED — where to write/read trace JSON files\n\
     \x20 --report PATH         REQUIRED for score-l1/all (not needed for a pure drive run) —\n\
     \x20                       where to write the score-l1 report JSON\n\
     \x20 --constrained         drive via single-shot constrained output (response_format\n\
     \x20                       json_schema, no tool loop) instead of the run_agent tool loop.\n\
     \x20                       #662 step 2 — bible slices only; traces carry constrained:true.\n\
     \x20 -h, --help            show this help\n"
        .to_string()
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
    constrained: bool,
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
            // Boolean flag — takes NO value, so advance by ONE (not the 2 the
            // value-carrying flags below the loop assume) and re-enter.
            "--constrained" => {
                flags.constrained = true;
                i += 1;
                continue;
            }
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

    // #662 defect 3: no compile-time-baked default — every mode needs
    // corpus/traces (drive writes traces, score-l1 reads both); --report
    // only matters once score-l1 actually runs.
    let corpus_dir = flags.corpus_dir.ok_or_else(|| {
        anyhow::anyhow!("--corpus-dir is required (see --help) — no built-in default")
    })?;
    let traces_dir = flags.traces_dir.ok_or_else(|| {
        anyhow::anyhow!("--traces-dir (or --out) is required (see --help) — no built-in default")
    })?;
    if matches!(mode, Mode::ScoreL1 | Mode::All) && flags.report_path.is_none() {
        anyhow::bail!("{:?} stage requires --report (see --help)", mode);
    }

    Ok(Args {
        mode,
        candidate_url: flags.candidate_url,
        candidate_model: flags.candidate_model,
        slice: flags.slice.filter(|s| s != "all"),
        corpus_dir,
        traces_dir,
        report_path: flags.report_path,
        constrained: flags.constrained,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> anyhow::Result<Args> {
        parse_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn corpus_dir_missing_is_a_clear_error() {
        let err = args(&["score-l1", "--traces-dir", "t", "--report", "r"])
            .expect_err("must require --corpus-dir");
        assert!(
            err.to_string().contains("--corpus-dir"),
            "error must name the missing flag: {err}"
        );
    }

    #[test]
    fn traces_dir_missing_is_a_clear_error() {
        let err = args(&["score-l1", "--corpus-dir", "c", "--report", "r"])
            .expect_err("must require --traces-dir");
        assert!(
            err.to_string().contains("--traces-dir"),
            "error must name the missing flag: {err}"
        );
    }

    #[test]
    fn report_required_for_score_l1_but_not_for_plain_drive() {
        let err = args(&["score-l1", "--corpus-dir", "c", "--traces-dir", "t"])
            .expect_err("score-l1 must require --report");
        assert!(
            err.to_string().contains("--report"),
            "error must name the missing flag: {err}"
        );

        let ok = args(&[
            "drive",
            "--candidate-url",
            "http://x.invalid",
            "--model",
            "m",
            "--corpus-dir",
            "c",
            "--traces-dir",
            "t",
        ])
        .expect("a plain drive run must not require --report");
        assert!(ok.report_path.is_none());
    }

    #[test]
    fn all_required_flags_present_parses_cleanly() {
        let a = args(&[
            "all",
            "--candidate-url",
            "http://x.invalid",
            "--model",
            "m",
            "--corpus-dir",
            "c",
            "--traces-dir",
            "t",
            "--report",
            "r",
        ])
        .expect("every required flag is present");
        assert_eq!(a.corpus_dir, PathBuf::from("c"));
        assert_eq!(a.traces_dir, PathBuf::from("t"));
        assert_eq!(a.report_path, Some(PathBuf::from("r")));
        assert!(!a.constrained, "constrained defaults to false");
    }

    #[test]
    fn constrained_is_a_valueless_flag_that_does_not_swallow_the_next_flag() {
        // #662 step 2: --constrained takes NO value; the flag AFTER it must
        // still parse (the parser must advance by 1, not 2, past it).
        let a = args(&[
            "drive",
            "--constrained",
            "--candidate-url",
            "http://x.invalid",
            "--model",
            "m",
            "--corpus-dir",
            "c",
            "--traces-dir",
            "t",
        ])
        .expect("constrained + following flags must parse");
        assert!(a.constrained);
        assert_eq!(a.candidate_url.as_deref(), Some("http://x.invalid"));
        assert_eq!(a.candidate_model.as_deref(), Some("m"));
    }
}
