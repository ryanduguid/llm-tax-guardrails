//! stdout and stderr as the Python release had them, failures included.
//!
//! stdout is line-buffered on a terminal, unbuffered with `PYTHONUNBUFFERED`,
//! and otherwise held in a 128 KiB buffer (CPython 3.15's
//! `io.DEFAULT_BUFFER_SIZE`) until it fills or the process exits, so a closed
//! stdout usually fails only at exit: status 120 after "Exception ignored while
//! flushing sys.stdout:". Build and digests caught an OSError at a print.
//! argparse ignored its own write errors.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};

static UNBUFFERED: LazyLock<bool> = LazyLock::new(python_unbuffered);
static STDOUT: LazyLock<Mutex<Box<dyn Write + Send>>> =
    LazyLock::new(|| Mutex::new(python_stdout(*UNBUFFERED)));
static COMMAND: OnceLock<&str> = OnceLock::new();
static OUTPUT_FAILED: Mutex<Option<String>> = Mutex::new(None);

/// A failed stderr write stayed pending in Python's buffered stderr.
static STDERR_FAILED: AtomicBool = AtomicBool::new(false);

/// Python text-mode output: newlines become CRLF on Windows.
fn native(text: &str) -> String {
    if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_string()
    }
}

/// `PYTHONUNBUFFERED` as CPython reads it: set and not an integer equal to 0.
fn python_unbuffered() -> bool {
    std::env::var("PYTHONUNBUFFERED").is_ok_and(|value| {
        !value.is_empty()
            && value
                .trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r'])
                .parse::<i32>()
                != Ok(0)
    })
}

fn python_stdout(unbuffered: bool) -> Box<dyn Write + Send> {
    use std::io::IsTerminal;
    let stdout = io::stdout();
    if stdout.is_terminal() {
        return Box::new(io::LineWriter::new(stdout));
    }
    if unbuffered {
        #[cfg(unix)]
        let handle = std::os::fd::AsFd::as_fd(&stdout)
            .try_clone_to_owned()
            .map(std::fs::File::from);
        #[cfg(windows)]
        let handle = std::os::windows::io::AsHandle::as_handle(&stdout)
            .try_clone_to_owned()
            .map(std::fs::File::from);
        return match handle {
            Ok(file) => Box::new(file),
            Err(_) => Box::new(FlushEach(stdout)),
        };
    }
    Box::new(io::BufWriter::with_capacity(128 * 1024, stdout))
}

/// Unbuffered stdout when the handle cannot be cloned: each write is flushed.
struct FlushEach(io::Stdout);

impl Write for FlushEach {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.0.write(buf)?;
        self.0.flush()?;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

fn stdout() -> MutexGuard<'static, Box<dyn Write + Send>> {
    STDOUT.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn write_stdout(text: &str) -> io::Result<()> {
    write_stdout_bytes(native(text).as_bytes())
}

pub(crate) fn write_stdout_bytes(bytes: &[u8]) -> io::Result<()> {
    stdout().write_all(bytes).inspect_err(|error| {
        *OUTPUT_FAILED.lock().unwrap_or_else(PoisonError::into_inner) = Some(stream_error(error));
    })
}

pub(crate) fn write_stderr(text: &str) -> io::Result<()> {
    write_stderr_bytes(native(text).as_bytes())
}

pub(crate) fn write_stderr_bytes(bytes: &[u8]) -> io::Result<()> {
    io::stderr()
        .lock()
        .write_all(bytes)
        .inspect_err(|_| STDERR_FAILED.store(true, Ordering::SeqCst))
}

/// The exception line of a failed write to a standard stream:
/// `OSError: [Errno 22] Invalid argument` for a closed pipe on Windows,
/// `BrokenPipeError: [Errno 32] Broken pipe` elsewhere.
fn stream_error(error: &io::Error) -> String {
    let mut text = error.to_string();
    if let Some(at) = text.rfind(" (os error ") {
        text.truncate(at);
    }
    match error.raw_os_error() {
        Some(112) if cfg!(windows) => "OSError: [Errno 28] No space left on device".into(),
        Some(_) if cfg!(windows) => "OSError: [Errno 22] Invalid argument".into(),
        Some(32) => format!("BrokenPipeError: [Errno 32] {text}"),
        Some(code) => format!("OSError: [Errno {code}] {text}"),
        None => format!("OSError: {text}"),
    }
}

pub(crate) fn command(name: &'static str) {
    let _ = COMMAND.set(name);
}

fn report_failure(exception: &str) {
    let message = match COMMAND.get().copied() {
        Some(command @ ("build" | "digests")) => {
            let detail = exception
                .split_once(": ")
                .map_or(exception, |(_, detail)| detail);
            format!("{command}: {detail}\n")
        }
        _ => format!("error: {exception}\n"),
    };
    let _ = write_stderr(&message);
}

fn uncaught(error: &io::Error) -> ! {
    OUTPUT_FAILED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    report_failure(&stream_error(error));
    std::process::exit(shutdown(1))
}

/// `print(text)`.
pub fn print(text: &str) {
    write_stdout(&format!("{text}\n")).unwrap_or_else(|e| uncaught(&e));
}

/// `print(text, file=sys.stderr)`.
pub fn eprint(text: &str) {
    write_stderr(&format!("{text}\n")).unwrap_or_else(|e| uncaught(&e));
}

/// argparse's help (stdout) or usage error (stderr), whose write errors it ignored.
pub fn argparse(error: &clap::Error) {
    // Match Clap's colour selection, including enabling ANSI on Windows.
    let choice = if error.use_stderr() {
        anstream::AutoStream::auto(io::stderr()).current_choice()
    } else {
        anstream::AutoStream::auto(io::stdout()).current_choice()
    };
    #[cfg(windows)]
    if choice == anstream::ColorChoice::Always {
        // Legacy Windows consoles need Clap's Wincon renderer.
        if error.print().is_err() && error.use_stderr() {
            STDERR_FAILED.store(true, Ordering::SeqCst);
        }
        return;
    }
    let text = if choice == anstream::ColorChoice::Never {
        error.to_string()
    } else {
        error.render().ansi().to_string()
    };
    let is_error = error.use_stderr();
    let _ = if is_error {
        write_stderr(&text)
    } else {
        write_stdout(&text)
    };
    if !is_error {
        // argparse ignored the immediate error; any pending bytes still fail
        // when shutdown flushes the buffered stream.
        OUTPUT_FAILED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

/// Interpreter shutdown: flush stdout, then stderr. Output still pending in
/// either fails there with exit status 120.
pub fn shutdown(code: i32) -> i32 {
    let mut code = code;
    if let Some(error) = OUTPUT_FAILED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    {
        report_failure(&error);
        code = 1;
    }
    if let Err(error) = stdout().flush() {
        let _ = write_stderr(&format!(
            "Exception ignored while flushing sys.stdout:\n{}\n",
            stream_error(&error)
        ));
        code = 120;
    }
    if STDERR_FAILED.load(Ordering::SeqCst) && !*UNBUFFERED {
        code = 120;
    }
    code
}

pub(crate) fn flush_stdout() -> io::Result<()> {
    stdout().flush()
}
