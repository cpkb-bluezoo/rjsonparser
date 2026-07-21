//! JSONTestSuite conformance (y_ must pass, n_ must fail, i_ either).

use rjsonparser::{DefaultHandler, Parser};
use std::fs;
use std::path::PathBuf;

fn suite_dir() -> PathBuf {
    // Prefer sibling Java repo corpus; fall back to vendored path if present.
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../jsonparser/JSONTestSuite/test_parsing"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("JSONTestSuite/test_parsing"),
    ];
    candidates
        .into_iter()
        .find(|p| p.is_dir())
        .unwrap_or_else(|| {
            panic!(
                "JSONTestSuite not found; expected at ../jsonparser/JSONTestSuite/test_parsing"
            )
        })
}

fn try_parse(bytes: &[u8]) -> Result<(), String> {
    let mut handler = DefaultHandler;
    let mut parser = Parser::new(&mut handler);
    parser.disable_all_limits();
    let mut data = bytes;
    parser
        .receive(&mut data)
        .map_err(|e| e.to_string())?;
    parser.close().map_err(|e| e.to_string())
}

#[test]
fn json_test_suite() {
    let dir = suite_dir();
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    entries.sort();

    assert!(
        !entries.is_empty(),
        "no .json files in {}",
        dir.display()
    );

    let mut failures = Vec::new();
    let mut y_ok = 0usize;
    let mut n_ok = 0usize;
    let mut i_count = 0usize;

    for path in &entries {
        let name = path.file_name().unwrap().to_string_lossy();
        let bytes = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let result = try_parse(&bytes);

        if name.starts_with("y_") {
            match result {
                Ok(()) => y_ok += 1,
                Err(e) => failures.push(format!("{name}: expected pass, got error: {e}")),
            }
        } else if name.starts_with("n_") {
            match result {
                Err(_) => n_ok += 1,
                Ok(()) => failures.push(format!("{name}: expected fail, but parsed OK")),
            }
        } else if name.starts_with("i_") {
            i_count += 1;
            // either outcome is fine
        }
    }

    if !failures.is_empty() {
        panic!(
            "JSONTestSuite failures ({}):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    eprintln!("JSONTestSuite: y_ok={y_ok} n_ok={n_ok} i_={i_count}");
    assert!(y_ok > 0 && n_ok > 0);
}
