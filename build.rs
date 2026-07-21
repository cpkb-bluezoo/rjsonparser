//! Generate one `#[test]` per JSONTestSuite file (https://github.com/nst/JSONTestSuite).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("json_test_suite_cases.rs");

    // Rebuild when the suite appears or changes (same layout as CI / fetch script).
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("JSONTestSuite").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("JSONTestSuite/test_parsing").display()
    );

    let candidates = [
        manifest.join("JSONTestSuite/test_parsing"),
        manifest.join("../JSONTestSuite/test_parsing"),
    ];

    let Some(suite_dir) = candidates.into_iter().find(|p| p.is_dir()) else {
        fs::write(
            &dest,
            r#"
#[test]
fn json_test_suite_not_found() {
    panic!(
        "JSONTestSuite not found. Run: ./scripts/fetch-json-test-suite.sh\n\
         (clones https://github.com/nst/JSONTestSuite into JSONTestSuite/)"
    );
}
"#,
        )
        .unwrap();
        return;
    };

    let mut entries: Vec<PathBuf> = fs::read_dir(&suite_dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", suite_dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    entries.sort();

    for path in &entries {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let mut code = String::new();
    for path in &entries {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("utf-8 file name");
        let expect = if name.starts_with("y_") {
            "Expect::Pass"
        } else if name.starts_with("n_") {
            "Expect::Fail"
        } else if name.starts_with("i_") {
            "Expect::Either"
        } else {
            continue;
        };
        let fn_name = sanitize_fn_name(name);
        let path_lit = path_raw_literal(path);
        code.push_str(&format!(
            "#[test]\nfn {fn_name}() {{\n    run_case({path_lit}, {expect});\n}}\n\n"
        ));
    }

    assert!(
        !code.is_empty(),
        "no y_/n_/i_ *.json files in {}",
        suite_dir.display()
    );

    fs::write(&dest, code).unwrap();
}

fn sanitize_fn_name(file_name: &str) -> String {
    let base = file_name.strip_suffix(".json").unwrap_or(file_name);
    let mut s = String::with_capacity(base.len() * 2);
    for c in base.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => s.push(c.to_ascii_lowercase()),
            '+' => s.push_str("_plus_"),
            '-' => s.push_str("_minus_"),
            '.' => s.push_str("_dot_"),
            _ => s.push('_'),
        }
    }
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    let s = s.trim_matches('_').to_string();
    if s.is_empty() || s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("case_{s}")
    } else {
        s
    }
}

fn path_raw_literal(path: &Path) -> String {
    let s = path.to_string_lossy();
    // Prefer raw string; fall back if it contains the closing delimiter sequence.
    if !s.contains("\"#") {
        format!("r#\"{s}\"#")
    } else {
        format!("{:?}", s.as_ref())
    }
}
