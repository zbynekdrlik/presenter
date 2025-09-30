use std::path::PathBuf;
use std::{env, process};

use anyhow::{Context, Result};
use presenter_importer::ProPresenterImporter;
use presenter_persistence::{DatabaseSettings, Repository};

const DEFAULT_DB_URL: &str = "sqlite://presenter_dev.db";
const DEFAULT_ROOT: &str = "Propresenter library";

#[derive(Debug)]
struct CliConfig {
    root: PathBuf,
    purge: bool,
}

impl CliConfig {
    fn parse() -> Result<Self> {
        let mut args = env::args().skip(1);
        let mut root = PathBuf::from(DEFAULT_ROOT);
        let mut purge = true;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--root" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--root requires a value"))?;
                    root = PathBuf::from(value);
                }
                "--purge" => purge = true,
                "--keep" | "--keep-existing" => purge = false,
                "--help" | "-h" => {
                    print_usage();
                    process::exit(0);
                }
                other => {
                    // Positional path override
                    root = PathBuf::from(other);
                }
            }
        }

        Ok(Self { root, purge })
    }
}

fn print_usage() {
    eprintln!("Usage: import_propresenter [--purge|--keep] [--root PATH] [ROOT_PATH]\n");
    eprintln!("Options:");
    eprintln!("  --purge            Remove libraries before importing (default)");
    eprintln!("  --keep             Retain existing libraries and append new ones");
    eprintln!("  --root PATH        Explicit path to the ProPresenter library root");
    eprintln!("  -h, --help         Show this help message");
    eprintln!("");
    eprintln!(
        "When ROOT_PATH is supplied as a positional argument it overrides --root, e.g.\n  import_propresenter /data/PropPresenter",
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = CliConfig::parse()?;
    let db_url = env::var("PRESENTER_DB_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());

    let repository = Repository::connect(&DatabaseSettings::new(&db_url))
        .await
        .with_context(|| format!("failed to connect to database at {db_url}"))?;

    if config.purge {
        repository
            .purge_presentation_content()
            .await
            .context("failed to purge existing ProPresenter content")?;
    }

    let importer = ProPresenterImporter::new(&repository);
    let summaries = importer.import_root(&config.root).await.with_context(|| {
        format!(
            "failed to import presentations from {}",
            config.root.display()
        )
    })?;

    if summaries.is_empty() {
        println!(
            "No ProPresenter libraries found under {}",
            config.root.display()
        );
    } else {
        for summary in summaries {
            println!(
                "Imported '{}' ({} presentations, {} slides) from {}",
                summary.library_name,
                summary.presentation_count,
                summary.slide_count,
                summary.source.display()
            );
        }
    }

    Ok(())
}
