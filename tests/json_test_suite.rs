//! One reported test per JSONTestSuite `test_parsing` file (discovered at runtime).
//!
//! No `build.rs` — suite loading only happens when this integration test binary runs.

use rjsonparser::{DefaultHandler, Parser};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Clone, Copy)]
enum Expect {
    Pass,
    Fail,
    Either,
}

fn suite_dir() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [
        manifest.join("JSONTestSuite/test_parsing"),
        manifest.join("../JSONTestSuite/test_parsing"),
    ]
    .into_iter()
    .find(|p| p.is_dir())
}

fn try_parse(bytes: &[u8]) -> Result<(), String> {
    let mut handler = DefaultHandler;
    let mut parser = Parser::new(&mut handler);
    parser.disable_all_limits();
    let mut data = bytes;
    parser.receive(&mut data).map_err(|e| e.to_string())?;
    parser.close().map_err(|e| e.to_string())
}

fn run_case(path: &Path, expect: Expect) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let result = try_parse(&bytes);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?");

    match expect {
        Expect::Pass => result.map_err(|e| format!("{name}: expected pass, got error: {e}")),
        Expect::Fail => {
            if result.is_ok() {
                Err(format!("{name}: expected fail, but parsed OK"))
            } else {
                Ok(())
            }
        }
        Expect::Either => Ok(()),
    }
}

fn expect_for(name: &str) -> Option<Expect> {
    if name.starts_with("y_") {
        Some(Expect::Pass)
    } else if name.starts_with("n_") {
        Some(Expect::Fail)
    } else if name.starts_with("i_") {
        Some(Expect::Either)
    } else {
        None
    }
}

fn collect_cases(dir: &Path) -> Vec<(String, PathBuf, Expect)> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    entries.sort();

    let mut cases = Vec::new();
    for path in entries {
        let name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("case")
            .to_owned();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let Some(expect) = expect_for(file_name) else {
            continue;
        };
        cases.push((name, path, expect));
    }
    cases
}

fn filter_matches(filter: &str, name: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    name.contains(filter)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let list = args.iter().any(|a| a == "--list");
    let filter = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .cloned()
        .unwrap_or_default();

    let Some(dir) = suite_dir() else {
        eprintln!(
            "JSONTestSuite not found. Run: ./scripts/fetch-json-test-suite.sh\n\
             (clones https://github.com/nst/JSONTestSuite into JSONTestSuite/)"
        );
        return ExitCode::from(101);
    };

    let cases = collect_cases(&dir);
    if cases.is_empty() {
        eprintln!("no y_/n_/i_ *.json files in {}", dir.display());
        return ExitCode::from(101);
    }

    if list {
        for (name, _, _) in &cases {
            if filter_matches(&filter, name) {
                println!("json_test_suite::{name}: test");
            }
        }
        return ExitCode::SUCCESS;
    }

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut filtered = 0usize;

    println!("\nrunning {} tests", cases.iter().filter(|(n, _, _)| filter_matches(&filter, n)).count());

    for (name, path, expect) in &cases {
        if !filter_matches(&filter, name) {
            filtered += 1;
            continue;
        }
        eprint!("test json_test_suite::{name} ... ");
        match run_case(path, *expect) {
            Ok(()) => {
                eprintln!("ok");
                passed += 1;
            }
            Err(e) => {
                eprintln!("FAILED");
                eprintln!("{e}");
                failed += 1;
            }
        }
    }

    let result = if failed == 0 { "ok" } else { "FAILED" };
    eprintln!(
        "\ntest result: {result}. {passed} passed; {failed} failed; 0 ignored; 0 measured; {filtered} filtered out"
    );

    if failed > 0 {
        ExitCode::from(101)
    } else {
        ExitCode::SUCCESS
    }
}
