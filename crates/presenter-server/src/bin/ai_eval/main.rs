//! `ai_eval` — #680's AI-eval harness driver + Layer-1 structural scorer.
//! Non-default binary (feature `ai-eval`). See `scripts/dev/ai-eval/README.md`
//! for the harness design and `corpus/SCHEMA.md` for the fixture schema.
//!
//! Thin dispatch only — every real behavior lives in a library module
//! (`corpus`, `trace`, `seed`, `drive`, `scorer`, `report`, `cli`, `logging`),
//! per this project's file/function size caps and the #680 design comment's
//! own "thin main + library modules" plan.

mod cli;
mod constrained;
mod corpus;
mod drive;
mod logging;
mod report;
mod scorer;
mod seed;
mod trace;

use cli::{Args, Mode};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match cli::parse_args(&raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ERROR: {e:#}");
            std::process::exit(1);
        }
    };

    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    // #688: build our OWN filter (base level + the known-benign per-case
    // boot/ingest noise directives) instead of the plain `fmt::init()` this
    // used to be — see `logging.rs` for exactly which lines are demoted and
    // why each is safe to demote. Falls back to the plain base level (no
    // demotion) if RUST_LOG somehow contains something the filter parser
    // rejects, rather than failing the whole harness over a logging nicety.
    let base_rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let filter = match logging::build_eval_log_filter(&base_rust_log) {
        Ok(filter) => filter,
        Err(err) => {
            eprintln!("WARNING: {err:#} — falling back to the base RUST_LOG with no demotion");
            tracing_subscriber::EnvFilter::new(base_rust_log)
        }
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    match args.mode {
        Mode::Drive => run_drive(&args).await,
        Mode::ScoreL1 => run_score_l1(&args),
        Mode::All => {
            run_drive(&args).await?;
            run_score_l1(&args)
        }
    }
}

async fn run_drive(args: &Args) -> anyhow::Result<()> {
    let cases = corpus::load_corpus(&args.corpus_dir, args.slice.as_deref())?;
    if cases.is_empty() {
        anyhow::bail!(
            "no corpus cases found under {} (slice: {})",
            args.corpus_dir.display(),
            args.slice.as_deref().unwrap_or("all")
        );
    }
    // Validated present by cli::parse_args for Drive/All.
    let candidate_url = args
        .candidate_url
        .as_deref()
        .expect("cli::parse_args guarantees --candidate-url for drive/all");
    let candidate_model = args
        .candidate_model
        .as_deref()
        .expect("cli::parse_args guarantees --model for drive/all");

    println!(
        "Driving {} case(s) against {} (model: {}){}",
        cases.len(),
        candidate_url,
        candidate_model,
        if args.constrained {
            " [CONSTRAINED single-shot mode]"
        } else {
            ""
        }
    );

    let mut traces = Vec::with_capacity(cases.len());
    for case in &cases {
        println!("  {} ({})...", case.id, case.slice);
        if let Some(desc) = case
            .setup
            .as_ref()
            .map(|s| &s.description)
            .filter(|d| !d.is_empty())
        {
            println!("    {desc}");
        }
        // #662 step 2: --constrained drives the single-shot constrained-output
        // path (response_format json_schema, no tool loop); default is the real
        // run_agent tool loop.
        let trace = if args.constrained {
            drive::drive_case_constrained(case, candidate_url, candidate_model).await
        } else {
            drive::drive_case(case, candidate_url, candidate_model).await
        };
        if trace.seed_failed {
            eprintln!(
                "    SEED FAILED: {}",
                trace.error.as_deref().unwrap_or("(no reason recorded)")
            );
        } else if let Some(err) = &trace.error {
            eprintln!("    WARNING: {err}");
        }
        trace.write_to(&args.traces_dir)?;
        traces.push(trace);
    }

    println!(
        "Wrote {} trace(s) to {}",
        cases.len(),
        args.traces_dir.display()
    );

    // #662 defect 1: ANY case that could not even be seeded is a harness/
    // environment problem, not model-evaluation data — this must stop the
    // run loudly, unconditionally (never just when EVERY case failed to
    // seed), or a partial seed failure silently degrades into a "model
    // scored X%" result at score-l1 with no visible signal at drive time.
    if let Some(report) = drive::seed_failure_report(&traces) {
        anyhow::bail!(report);
    }

    // Every single case erroring out (as a genuine candidate/model error,
    // seeding already ruled out above) is almost certainly an operational
    // problem (candidate endpoint unreachable, bad --model), not 100%
    // genuine model-quality failures — surface that distinctly instead of
    // silently reporting a "0% pass" that would masquerade as eval data.
    let run_error_count = traces
        .iter()
        .filter(|t| t.error.is_some() && !t.seed_failed)
        .count();
    if run_error_count == cases.len() {
        anyhow::bail!(
            "all {run_error_count} case(s) errored before producing a response — check \
             --candidate-url ({candidate_url}) is reachable and --model ({candidate_model}) is valid"
        );
    }

    Ok(())
}

fn run_score_l1(args: &Args) -> anyhow::Result<()> {
    let cases = corpus::load_corpus(&args.corpus_dir, args.slice.as_deref())?;
    if cases.is_empty() {
        anyhow::bail!(
            "no corpus cases found under {} (slice: {})",
            args.corpus_dir.display(),
            args.slice.as_deref().unwrap_or("all")
        );
    }
    let traces = trace::load_traces(&args.traces_dir)?;
    if traces.is_empty() {
        anyhow::bail!(
            "no traces under {} — run the drive stage first",
            args.traces_dir.display()
        );
    }

    let mut results = Vec::with_capacity(cases.len());
    for case in &cases {
        let Some(t) = traces.iter().find(|t| t.case_id == case.id) else {
            eprintln!(
                "WARNING: no trace found for case {} — skipping (run drive first)",
                case.id
            );
            continue;
        };
        let score = scorer::score_trace(case, t);
        results.push((case, t, score));
    }

    // Validated present by cli::parse_args for ScoreL1/All.
    let report_path = args
        .report_path
        .as_deref()
        .expect("cli::parse_args guarantees --report for score-l1/all");

    let report = report::build_report(&results);
    report::print_summary(&report);
    report::write_report(&report, report_path)?;
    println!("\nWrote report to {}", report_path.display());

    // score-l1 REPORTS the Layer-1 pass/fail summary; it does not apply
    // report §6.5's bar or decide overall gate success — that is the
    // (not-yet-built) `gate` stage's job (report §8 step 11), which
    // consumes this exact results.json. A non-zero exit here means
    // scoring itself could not run to completion, not "some cases failed".
    Ok(())
}
