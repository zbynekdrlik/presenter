use anyhow::Context;
use presenter_bible::default_translation_specs;
use presenter_importer::bible::BibleIngestionService;
use presenter_persistence::{DatabaseSettings, Repository};
use std::env;

const DEFAULT_DB_URL: &str = "sqlite://presenter_dev.db";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db_url = env::var("PRESENTER_DB_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let repository = Repository::connect(&DatabaseSettings::new(&db_url))
        .await
        .with_context(|| format!("failed to connect to database at {db_url}"))?;
    let ingestion = BibleIngestionService::with_http(&repository)?;

    // Filter to only available specs (URL sources always available,
    // LocalFile sources only when env var is set)
    let all_specs = default_translation_specs();
    let available_specs: Vec<_> = all_specs
        .into_iter()
        .filter(|spec| {
            if spec.source.is_available() {
                true
            } else {
                println!(
                    "Skipping {} (local file not configured)",
                    spec.translation.code
                );
                false
            }
        })
        .collect();

    let mut success_count = 0;
    let mut fail_count = 0;

    for spec in available_specs {
        let code = spec.translation.code.clone();
        match ingestion.ingest(&spec).await {
            Ok(summary) => {
                println!(
                    "Imported {count} passages for translation {code}",
                    count = summary.passage_count,
                    code = summary.translation_code
                );
                success_count += 1;
            }
            Err(err) => {
                eprintln!("Warning: failed to import {code}: {err:#}");
                fail_count += 1;
            }
        }
    }

    if success_count == 0 && fail_count > 0 {
        anyhow::bail!("all {fail_count} bible translations failed to import");
    }

    if fail_count > 0 {
        eprintln!(
            "Imported {success_count} translations, {fail_count} failed (see warnings above)"
        );
    }

    Ok(())
}
