use presenter_importer::load_presentation_from_path;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let path = PathBuf::from(std::env::args().nth(1).expect("path"));
    let presentation = load_presentation_from_path(&path)?;
    println!(
        "name: {} slides: {}",
        presentation.name,
        presentation.slides.len()
    );
    for (idx, slide) in presentation.slides.iter().enumerate() {
        println!(
            "{} (order {}) | main='{}' translation='{}' stage='{}'",
            idx,
            slide.order,
            slide.content.main.value(),
            slide.content.translation.value(),
            slide.content.stage.value()
        );
    }
    Ok(())
}
