//! Embeds the bundled module catalog: every `modules/<category>/<id>/`
//! folder that holds a `module.toml` becomes one entry with all its files.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    out.sort();
    out
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("modules");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut code = String::from("pub(crate) static BUNDLED: &[BundledModule] = &[\n");
    for category in sorted_dirs(&root).into_iter().filter(|p| p.is_dir()) {
        println!("cargo:rerun-if-changed={}", category.display());
        for module in sorted_dirs(&category).into_iter().filter(|p| p.is_dir()) {
            println!("cargo:rerun-if-changed={}", module.display());
            let manifest = module.join("module.toml");
            if !manifest.is_file() {
                continue;
            }
            let _ = writeln!(
                code,
                "    BundledModule {{\n        path: {:?},\n        manifest: include_str!({:?}),\n        files: &[",
                format!(
                    "{}/{}",
                    category.file_name().unwrap().to_string_lossy(),
                    module.file_name().unwrap().to_string_lossy()
                ),
                manifest.display().to_string(),
            );
            for file in sorted_dirs(&module) {
                let name = file.file_name().unwrap().to_string_lossy().into_owned();
                if !file.is_file() || name == "module.toml" {
                    continue;
                }
                println!("cargo:rerun-if-changed={}", file.display());
                let _ = writeln!(
                    code,
                    "            ({name:?}, include_str!({:?})),",
                    file.display().to_string()
                );
            }
            code.push_str("        ],\n    },\n");
        }
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("bundled.rs");
    std::fs::write(out, code).expect("write bundled.rs");
}
