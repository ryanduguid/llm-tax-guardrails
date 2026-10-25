//! The failures the original builder distinguished, by the Python exception
//! that carried them. Each command reports or escalates them as the Python
//! command did, so a malformed input and a checker that fell over keep their
//! separate exit codes.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Fail {
    /// `ModelError`: a source or record that does not meet its contract.
    Model(String),
    /// `BuildError`: the tree cannot be built at all.
    Build(String),
    /// `OSError`, already rendered as Python renders it.
    Os(String),
    /// `UnicodeDecodeError`, already rendered.
    Decode(String),
    /// `HarnessError`: a responses file the scorer refuses.
    Harness(String),
    /// `BundleError`: a bundle that fails its approved manifest.
    Bundle(String),
    /// Any other exception, which the Python builder let escape as a traceback.
    Internal(String),
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fail::Model(m)
            | Fail::Build(m)
            | Fail::Os(m)
            | Fail::Decode(m)
            | Fail::Harness(m)
            | Fail::Bundle(m) => f.write_str(m),
            Fail::Internal(m) => write!(f, "unexpected error: {m}"),
        }
    }
}

impl Fail {
    /// Python's `ValueError` family, which `except ValueError` also catches.
    pub fn is_value_error(&self) -> bool {
        matches!(self, Fail::Decode(_) | Fail::Bundle(_))
    }
}
