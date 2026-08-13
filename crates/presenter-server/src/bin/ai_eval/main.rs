//! `ai_eval` — #680's AI-eval harness driver + Layer-1 structural scorer.
//! Non-default binary (feature `ai-eval`). See `scripts/dev/ai-eval/README.md`
//! for the harness design and `corpus/SCHEMA.md` for the fixture schema.
//!
//! Thin dispatch only — every real behavior lives in a library module
//! (`corpus`, `trace`, `seed`, `drive`, `scorer`, `report`, `cli`), per this
//! project's file/function size caps and the #680 design comment's own
//! "thin main + library modules" plan.

mod cli;
mod corpus;
mod drive;
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
    tracing_subscriber::fmt::init();

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
        "Driving {} case(s) against {} (model: {})",
        cases.len(),
        candidate_url,
        candidate_model
    );

    let mut error_count = 0usize;
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
        let trace = drive::drive_case(case, candidate_url, candidate_model).await;
        if let Some(err) = &trace.error {
            eprintln!("    WARNING: {err}");
            error_count += 1;
        }
        trace.write_to(&args.traces_dir)?;
    }

    println!(
        "Wrote {} trace(s) to {}",
        cases.len(),
        args.traces_dir.display()
    );

    // Every single case erroring out is almost certainly an operational
    // problem (candidate endpoint unreachable, bad --model), not 100%
    // genuine model-quality failures — surface that distinctly instead of
    // silently reporting a "0% pass" that would masquerade as eval data.
    if error_count == cases.len() {
        anyhow::bail!(
            "all {error_count} case(s) errored before producing a response — check \
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
        results.push((case, score));
    }

    let report = report::build_report(&results);
    report::print_summary(&report);
    report::write_report(&report, &args.report_path)?;
    println!("\nWrote report to {}", args.report_path.display());

    // score-l1 REPORTS the Layer-1 pass/fail summary; it does not apply
    // report §6.5's bar or decide overall gate success — that is the
    // (not-yet-built) `gate` stage's job (report §8 step 11), which
    // consumes this exact results.json. A non-zero exit here means
    // scoring itself could not run to completion, not "some cases failed".
    Ok(())
}
