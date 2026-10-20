//! The YAML and JSON readers against a table of how the Python builder read
//! the same texts. `tests/data/python_readers.json` was recorded once from the
//! base-revision builder (PyYAML 6.0.3, CPython's json scanner): for each text,
//! the loaded value's repr, the refusal message, or the name of the exception
//! the Python builder let escape.

use drdebits::error::Fail;
use drdebits::model::read_bytes;
use drdebits::py::PyPath;
use drdebits::pyjson::{self, Json};

fn table() -> Json {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/python_readers.json");
    pyjson::loads(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn cases(table: &Json, key: &str) -> Vec<Json> {
    match table.get(key) {
        Some(Json::Array(items)) => items.clone(),
        _ => panic!("missing {key}"),
    }
}

fn text(case: &Json, key: &str) -> String {
    case.get(key).and_then(Json::as_str).unwrap().to_string()
}

#[test]
fn yaml_sources_read_as_the_python_builder_read_them() {
    let failures = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            let table = table();
            let mut failures = Vec::new();
            for case in cases(&table, "yaml") {
                let source = text(&case, "yaml");
                let got = read_bytes(&PyPath::new("source.yaml"), source.as_bytes());
                // Documented divergence: Python kept a lone surrogate escape in
                // the string and failed only when writing it out; the port
                // refuses it when read.
                let lone_surrogate = case.get("value").and_then(Json::as_str).is_some_and(|v| {
                    [
                        "\\ud8", "\\ud9", "\\uda", "\\udb", "\\udc", "\\udd", "\\ude", "\\udf",
                    ]
                    .iter()
                    .any(|s| v.contains(s))
                });
                if lone_surrogate {
                    if !matches!(got, Err(Fail::Internal(_))) {
                        failures.push(format!(
                            "{source:?}: a lone surrogate must be refused, got {got:?}"
                        ));
                    }
                    continue;
                }
                let ok = match (
                    &got,
                    case.get("value"),
                    case.get("error"),
                    case.get("internal"),
                ) {
                    (Ok(value), Some(expected), _, _) => {
                        value.repr().ok().as_deref() == expected.as_str()
                    }
                    (Err(Fail::Model(message)), _, Some(expected), _) => {
                        Some(message.as_str()) == expected.as_str()
                    }
                    (Err(Fail::Internal(_)), _, _, Some(_)) => true,
                    _ => false,
                };
                if !ok {
                    failures.push(format!("{source:?}\n  expected {case:?}\n  got {got:?}"));
                }
            }
            failures
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(
        failures.is_empty(),
        "{} of the recorded texts differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn json_records_read_as_the_python_builder_read_them() {
    let table = table();
    let mut failures = Vec::new();
    for case in cases(&table, "json") {
        let source = text(&case, "json");
        let got = pyjson::loads(&source);
        let ok = match (&got, case.get("value"), case.get("error")) {
            (Ok(value), Some(expected), _) => Some(value.repr().as_str()) == expected.as_str(),
            (Err(error), _, Some(expected)) => Some(error.0.as_str()) == expected.as_str(),
            _ => false,
        };
        if !ok {
            failures.push(format!("{source:?}: expected {case:?}, got {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
