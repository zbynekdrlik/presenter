use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = PathBuf::from("vendor/ProPresenter7-Proto/proto");
    if !proto_root.exists() {
        println!(
            "cargo:warning=ProPresenter proto sources not found at {}",
            proto_root.display()
        );
        return Ok(());
    }

    println!("cargo:rerun-if-changed=build.rs");
    for entry in walkdir::WalkDir::new(&proto_root) {
        let entry = entry?;
        if entry.file_type().is_file() {
            println!("cargo:rerun-if-changed={}", entry.path().display());
        }
    }

    let proto_files = vec![
        "presentation.proto",
        "presentationSlide.proto",
        "slide.proto",
        "graphicsData.proto",
        "action.proto",
        "cue.proto",
        "groups.proto",
        "background.proto",
        "effects.proto",
        "basicTypes.proto",
        "alignmentGuide.proto",
        "hotKey.proto",
        "stage.proto",
        "timers.proto",
        "layers.proto",
        "baseDocument.proto",
    ]
    .into_iter()
    .map(|name| proto_root.join(name))
    .collect::<Vec<_>>();

    let mut config = prost_build::Config::new();
    config.out_dir(PathBuf::from(env::var("OUT_DIR")?));
    config.compile_protos(&proto_files, &[proto_root])?;

    Ok(())
}
