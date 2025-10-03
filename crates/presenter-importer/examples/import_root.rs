use std::path::PathBuf;

use presenter_importer::{default_library_root, ProPresenterImporter};
use presenter_persistence::{DatabaseSettings, Repository};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db_url = std::env::var("PRESENTER_DB_URL")
        .unwrap_or_else(|_| "sqlite://presenter_dev.db".to_string());
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_library_root);

    let repo = Repository::connect(&DatabaseSettings::new(&db_url)).await?;
    let importer = ProPresenterImporter::new(&repo);
    let summaries = importer.import_root(&root).await?;

    if summaries.is_empty() {
        println!("No ProPresenter libraries found under {}", root.display());
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
