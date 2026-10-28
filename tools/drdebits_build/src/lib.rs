//! drdebits_build: build, verify and evaluate the DrDebits guide.
//!
//! `build` regenerates committed outputs in place; `verify` checks them;
//! `digests` prints the guide and case-export digests an evaluation record
//! must carry, reading the sources and writing nothing.
//!
//! `verify` answers in three states, not two:
//!
//! ```text
//! 0   every check passed
//! 1   a check failed: the committed outputs do not match their sources
//! 2   the checks did not run, so nothing was checked
//! ```
//!
//! Without the third state a checker that fell over would exit with the
//! status a real finding uses, and a reader of CI would treat it as a checker
//! that found something. Inputs that cannot be read stay findings; the 2 band
//! is for the checks themselves failing to run.

/// One line to stdout, written as Python's text-mode `print` writes it.
macro_rules! outln {
    ($($arg:tt)*) => {{
        $crate::stdio::print(&format!($($arg)*));
    }};
}

/// One line to stderr, written as Python's text-mode `print` writes it.
macro_rules! errln {
    ($($arg:tt)*) => {{
        $crate::stdio::eprint(&format!($($arg)*));
    }};
}

pub mod builder;
pub mod bundle;
pub mod compare;
pub mod error;
pub mod evals;
pub mod fuzz;
pub mod gate;
pub mod harness;
pub mod linkcheck;
pub mod model;
pub mod net;
pub mod py;
pub mod pyjson;
pub mod pyunicode;
pub mod quotecheck;
pub mod sources;
mod stdio;
pub mod templates;
pub mod verify;
pub mod yaml;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use error::Fail;
use py::PyPath;

/// A standard stream in Python's text mode: each `\n` is written as the
/// platform line ending, so output bytes match the Python builder's on
/// Windows as well as elsewhere.
pub struct PyText {
    stderr: bool,
}

impl PyText {
    pub fn stdout() -> Self {
        PyText { stderr: false }
    }

    pub fn stderr() -> Self {
        PyText { stderr: true }
    }
}

/// `buf` as text mode writes it here: on Windows each `\n` becomes `\r\n`.
#[cfg(windows)]
fn text_mode(buf: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if buf.contains(&b'\n') {
        String::from_utf8_lossy(buf)
            .replace('\n', "\r\n")
            .into_bytes()
            .into()
    } else {
        buf.into()
    }
}

/// `buf` as text mode writes it here: unchanged.
#[cfg(not(windows))]
fn text_mode(buf: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    buf.into()
}

impl std::io::Write for PyText {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let bytes = text_mode(buf);
        if self.stderr {
            stdio::write_stderr_bytes(&bytes)?;
        } else {
            stdio::write_stdout_bytes(&bytes)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.stderr {
            std::io::stderr().flush()
        } else {
            stdio::flush_stdout()
        }
    }
}

/// See the module documentation: 2 is "not checked", never "checked and clean".
const EXIT_OK: u8 = 0;
const EXIT_CHECKS_FAILED: u8 = 1;
const EXIT_COULD_NOT_RUN: u8 = 2;

/// What `verify` checked, so a passing line cannot be read as a wider
/// clearance than the checks give. Source currency is a separate command.
const VERIFY_OK_LINE: &str =
    "verify: OK (structure, version copies and review window; not a source-currency check)";

#[derive(Parser)]
#[command(
    name = "drdebits_build",
    version,
    about = "Build, verify and evaluate the DrDebits guide"
)]
struct Cli {
    /// The DrDebits checkout; found above the current directory when omitted.
    #[arg(long, global = true)]
    root: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Regenerate every committed output from src/.
    Build,
    /// Check the committed outputs against their sources.
    Verify,
    /// Print the guide and case-export digests for the tree as it stands.
    Digests,
    /// Compare two recorded evaluations without assigning verdicts.
    Compare(compare::Args),
    /// Install or verify an approved release bundle.
    Bundle(bundle::Args),
    /// Check that every link in the guide and its references still resolves.
    Linkcheck(linkcheck::Args),
    /// Check the pinned Federal Register compilations are still current.
    Sources(sources::Args),
    /// Score a responses file into a model-proposed observation record.
    Score(harness::ScoreArgs),
    /// Print the mechanical checks for each response in a responses file.
    Assess(harness::AssessArgs),
    /// Read a source in one explicit mode, without network access.
    ReadSource(harness::ReadSourceArgs),
    /// Assess whether a quote appears in supplied source text.
    Quote(quotecheck::Args),
    /// Check a high-impact draft against the output contract and reperform its arithmetic.
    Gate(gate::Args),
}

/// The root a command works in: `--root`, or the DrDebits tree above the current directory.
pub fn root_for(root: Option<&str>) -> Result<PyPath, Fail> {
    match root {
        Some(r) => Ok(PyPath::new(r)),
        None => {
            let cwd = std::env::current_dir().map_err(|e| Fail::Os(e.to_string()))?;
            builder::find_root(&cwd)
        }
    }
}

/// Report an error the Python builder did not catch, which it showed as a traceback.
fn emit_internal(command: &str, error: &Fail) {
    errln!("{}", internal_line(command, error));
}

/// The line `emit_internal` prints, naming the Python exception class.
fn internal_line(command: &str, error: &Fail) -> String {
    match error {
        Fail::Internal(_) => format!("{command}: {error}"),
        Fail::Decode(m) => format!("{command}: unexpected error: UnicodeDecodeError: {m}"),
        Fail::Os(m) => format!("{command}: unexpected error: OSError: {m}"),
        other => format!("{command}: unexpected error: {other}"),
    }
}

fn run_build(root: &PyPath) -> u8 {
    let result = (|| -> Result<(), Fail> {
        let s = builder::load_sources(root)?;
        // Render everything that does not read the stamped files, and discard
        // it. The stamped files are written first because SHA256SUMS covers
        // them, so a malformed evaluation record must fail the command before
        // README.md, MAINTENANCE.md and llms.txt change.
        builder::render_generated(&s);
        evals::build_results_md(root, &s)?;
        for rel in builder::STAMPED_FILES {
            let p = root.join(rel);
            if p.fs().is_file() {
                let stamped =
                    builder::stamp_version(&builder::read_text(&p)?, s.meta("guide_version"))?;
                std::fs::write(p.fs(), stamped.as_bytes())
                    .map_err(|e| Fail::Os(py::oserror_str(&e, &p.display(), py::OsStyle::Errno)))?;
                outln!("stamped {rel}");
            }
        }
        for rel in builder::write_outputs(root, root, &s)? {
            outln!("wrote {rel}");
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            if matches!(e, Fail::Internal(_)) {
                emit_internal("build", &e);
            } else {
                errln!("build: {e}");
            }
            1
        }
    }
}

fn run_digests(root: &PyPath) -> u8 {
    match builder::load_sources(root) {
        Ok(s) => {
            for (key, digest) in builder::build_digests(&s) {
                outln!("{key}: {digest}");
            }
            0
        }
        Err(e @ (Fail::Model(_) | Fail::Build(_) | Fail::Os(_))) => {
            errln!("digests: {e}");
            1
        }
        Err(e) => {
            emit_internal("digests", &e);
            1
        }
    }
}

fn run_verify(root: &PyPath) -> u8 {
    match verify::run_verify(root, py::Date::today()) {
        Ok(failures) => {
            for f in &failures {
                errln!("{f}");
            }
            if failures.is_empty() {
                outln!("{VERIFY_OK_LINE}");
                EXIT_OK
            } else {
                EXIT_CHECKS_FAILED
            }
        }
        Err(e) => {
            emit_internal("verify", &e);
            errln!(
                "verify: VERIFIER ERROR: the checks did not run, so these outputs are unverified, not verified"
            );
            EXIT_COULD_NOT_RUN
        }
    }
}

fn dispatch(cli: Cli) -> u8 {
    match cli.command {
        Command::Build | Command::Verify | Command::Digests => {
            let name = match cli.command {
                Command::Build => "build",
                Command::Verify => "verify",
                _ => "digests",
            };
            // Root discovery is the first thing a user hits from outside a
            // DrDebits tree, so it produces a clean message. For verify it
            // lands in the 2 band: no root means the checks never started.
            let root = match root_for(cli.root.as_deref()) {
                Ok(root) => root,
                Err(e) => {
                    errln!("{name}: {e}");
                    return if name == "verify" {
                        EXIT_COULD_NOT_RUN
                    } else {
                        1
                    };
                }
            };
            match cli.command {
                Command::Build => run_build(&root),
                Command::Verify => run_verify(&root),
                _ => run_digests(&root),
            }
        }
        Command::Compare(args) => compare::main(cli.root.as_deref(), args),
        Command::Bundle(args) => bundle::main(args),
        Command::Linkcheck(args) => linkcheck::main(cli.root.as_deref(), args),
        Command::Sources(args) => sources::main(cli.root.as_deref(), args),
        Command::Score(args) => harness::score_main(cli.root.as_deref(), args),
        Command::Assess(args) => harness::assess_main(cli.root.as_deref(), args),
        Command::ReadSource(args) => harness::read_source_main(args),
        Command::Quote(args) => quotecheck::main(args),
        Command::Gate(args) => gate::main(cli.root.as_deref(), args),
    }
}

/// Run the command line; the binary is a thin wrapper around this.
pub fn run() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            stdio::argparse(&error);
            return ExitCode::from(stdio::shutdown(error.exit_code()) as u8);
        }
    };
    stdio::command(match &cli.command {
        Command::Build => "build",
        Command::Digests => "digests",
        _ => "other",
    });
    // The YAML composer recurses once per nesting level, up to the Python
    // builder's own limit; a large stack keeps that well clear of overflow.
    let worker = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || dispatch(cli));
    let code = worker
        .expect("spawn the worker thread")
        .join()
        .unwrap_or(EXIT_COULD_NOT_RUN);
    ExitCode::from(stdio::shutdown(i32::from(code)) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn text_mode_translates_newlines_only_on_windows() {
        let expected: &[u8] = if cfg!(windows) { b"a\r\nb" } else { b"a\nb" };
        assert_eq!(&*text_mode(b"a\nb"), expected);
        assert_eq!(&*text_mode(b"no break \xff"), b"no break \xff");
    }

    #[test]
    fn both_streams_flush() {
        assert!(PyText::stdout().flush().is_ok());
        assert!(PyText::stderr().flush().is_ok());
    }

    #[test]
    fn uncaught_errors_name_the_python_exception() {
        let line = |e: Fail| internal_line("verify", &e);
        assert_eq!(
            line(Fail::Internal("ValueError: x".into())),
            "verify: unexpected error: ValueError: x"
        );
        assert_eq!(
            line(Fail::Decode("bad byte".into())),
            "verify: unexpected error: UnicodeDecodeError: bad byte"
        );
        assert_eq!(
            line(Fail::Os("[Errno 13] Permission denied: 'x'".into())),
            "verify: unexpected error: OSError: [Errno 13] Permission denied: 'x'"
        );
        assert_eq!(
            line(Fail::Model("src/x: bad".into())),
            "verify: unexpected error: src/x: bad"
        );
    }
}
