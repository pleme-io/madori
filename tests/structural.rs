use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

fn calls_in(roots: &[&str], needle: &str) -> Vec<String> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for root in roots {
        let dir = base.join(root);
        if dir.is_dir() {
            rust_files(&dir, &mut files);
        }
    }
    files.sort();
    let mut calls = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("a readable source file");
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            if code.contains(needle) {
                let rel = file.strip_prefix(base).unwrap_or(&file);
                calls.push(format!("{}:{}", rel.display(), n + 1));
            }
        }
    }
    calls
}

#[test]
fn the_surface_is_acquired_only_where_the_visible_token_is_spent() {
    let calls = calls_in(&["src", "examples"], ".get_current_texture(");
    assert_eq!(
        calls.len(),
        1,
        "get_current_texture is called only by Surface::acquire, which takes the Visible token: {calls:?}"
    );
    assert!(
        calls[0].starts_with("src/pacer/mod.rs:"),
        "the one call is the Surface impl's: {calls:?}"
    );
}
