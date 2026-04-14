use anyhow::Context;
use presenter_bible::{default_translation_specs, read_local_file_bytes};
use presenter_importer::bible::BibleIngestionService;
use presenter_importer::{compute_source_digest, PARSER_VERSION};
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

    // Only specs whose source is actually available (URL always, LocalFile when env var set)
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

    let mut imported = 0usize;
    let mut skipped = 0usize;

    for spec in available_specs {
        let code = spec.translation.code.clone();

        // Read the source bytes ourselves so we can hash once.
        let bytes = match read_local_file_bytes(&spec) {
            Ok(b) => b,
            Err(err) => {
                // URL-based specs (none today in production) — fall back to normal ingest,
                // which fetches via the scraper. No digest optimization for those.
                println!(
                    "read_local_file_bytes failed for {code} ({err}); falling back to network ingest",
                );
                let summary = ingestion
                    .ingest(&spec)
                    .await
                    .with_context(|| format!("bible ingestion failed for {code}"))?;
                println!(
                    "Imported {count} passages for translation {code}",
                    count = summary.passage_count,
                    code = summary.translation_code,
                );
                imported += 1;
                continue;
            }
        };

        let candidate_digest = compute_source_digest(&bytes, PARSER_VERSION);
        let stored_digest = repository
            .get_bible_source_digest(&code)
            .await
            .with_context(|| format!("failed to query stored digest for {code}"))?;

        if stored_digest.as_ref() == Some(&candidate_digest) {
            println!(
                "Skipping {code} (up-to-date, digest {short})",
                short = &candidate_digest[..12],
            );
            skipped += 1;
            continue;
        }

        let summary = ingestion
            .ingest_with_bytes(&spec, &bytes)
            .await
            .with_context(|| format!("bible ingestion failed for {code}"))?;
        repository
            .set_bible_source_digest(&code, &candidate_digest)
            .await
            .with_context(|| format!("failed to persist digest for {code}"))?;
        println!(
            "Imported {count} passages for translation {code} (digest {short})",
            count = summary.passage_count,
            code = summary.translation_code,
            short = &candidate_digest[..12],
        );
        imported += 1;
    }

    println!("Bible import done: {imported} imported, {skipped} skipped (up-to-date).");

    Ok(())
}
