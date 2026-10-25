//! The fuzz target's checks, shared by the cargo-fuzz target and the tests
//! that replay its seed corpus.
//!
//! Fabricated bytes go to the decision-status reader and to the YAML source
//! reader. Every status the decision reader returns must be empty or one the
//! guide allows; the source reader must return a mapping or refuse with a
//! finding. An unexpected failure, which the Python builder let escape as an
//! exception, fails the run too.

use crate::error::Fail;
use crate::harness::decision_statuses;
use crate::model::{self, ALLOWED_STATUSES};
use crate::py::PyPath;
use crate::yaml::Value;

/// Check one input; the error says which invariant failed.
pub fn check_input(data: &[u8]) -> Result<(), String> {
    check_with(data, decision_statuses, |bytes| {
        model::read_bytes(&PyPath::new("source.yaml"), bytes)
    })
}

/// `check_input` with the two readers supplied, so the tests can prove each check bites.
pub fn check_with(
    data: &[u8],
    statuses: impl Fn(&str) -> Vec<String>,
    read: impl Fn(&[u8]) -> Result<Value, Fail>,
) -> Result<(), String> {
    let text = String::from_utf8_lossy(data);
    for status in statuses(&text) {
        if !status.is_empty() && !ALLOWED_STATUSES.contains(&status.as_str()) {
            return Err(format!(
                "decision reader returned unknown status {}",
                crate::py::repr_str(&status)
            ));
        }
    }
    match read(data) {
        Ok(Value::Dict(_)) | Err(Fail::Model(_)) => Ok(()),
        Ok(_) => Err("the reader returned a non-mapping without refusing it".into()),
        Err(other) => Err(format!("the reader failed unexpectedly: {other}")),
    }
}
