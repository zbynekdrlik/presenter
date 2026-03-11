use std::fs;
use std::path::PathBuf;

fn main() {
    // Ensure the presenter-ui dist directory exists so include_dir! doesn't fail.
    // When Trunk hasn't been run yet, an empty dist/ is created as a placeholder.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let dist_dir = PathBuf::from(&manifest_dir)
        .join("..")
        .join("presenter-ui")
        .join("dist");

    if !dist_dir.exists() {
        fs::create_dir_all(&dist_dir).expect("Failed to create presenter-ui/dist directory");
    }

    println!("cargo:rerun-if-changed=build.rs");
    // Rebuild when WASM assets change
    println!(
        "cargo:rerun-if-changed={}",
        dist_dir.join("index.html").display()
    );
}
