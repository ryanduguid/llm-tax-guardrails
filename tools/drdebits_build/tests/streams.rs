//! A lost report cannot be a successful check. These cases were compared with
//! the Python release, including its buffered shutdown and argparse behaviour.
mod common;

use std::process::{Command, Stdio};

fn closed_pipe() -> Stdio {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    writer.into()
}

fn run(args: &[&str], stdout: bool, unbuffered: bool) -> (i32, String) {
    let tree = common::Tree::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_drdebits_build"));
    command
        .args(args)
        .current_dir(tree.path())
        .env_remove("PYTHONUNBUFFERED");
    if unbuffered {
        command.env("PYTHONUNBUFFERED", "1");
    }
    let result = if stdout {
        command.stdout(closed_pipe()).stderr(Stdio::piped())
    } else {
        command.stdout(Stdio::piped()).stderr(closed_pipe())
    }
    .output()
    .unwrap();
    let other = if stdout { result.stderr } else { result.stdout };
    (
        result.status.code().unwrap(),
        String::from_utf8(other).unwrap().replace("\r\n", "\n"),
    )
}

const ERROR: &str = if cfg!(windows) {
    "OSError: [Errno 22] Invalid argument"
} else {
    "BrokenPipeError: [Errno 32] Broken pipe"
};

#[test]
fn lost_buffered_report_fails_at_shutdown() {
    let message = format!("Exception ignored while flushing sys.stdout:\n{ERROR}\n");
    assert_eq!(run(&["digests"], true, false), (120, message.clone()));
    assert_eq!(run(&["--help"], true, false), (120, message));
}

#[test]
fn unbuffered_print_uses_the_command_error_and_help_ignores_its_write_error() {
    let detail = ERROR.split_once(": ").unwrap().1;
    assert_eq!(
        run(&["digests"], true, true),
        (1, format!("digests: {detail}\n"))
    );
    assert_eq!(run(&["--help"], true, true), (0, String::new()));
}

#[test]
fn failed_diagnostic_overrides_the_command_status() {
    assert_eq!(
        run(&["verify", "--root", "missing"], false, false),
        (120, String::new())
    );
    assert_eq!(
        run(&["verify", "--root", "missing"], false, true),
        (1, String::new())
    );
    assert_eq!(run(&["--bogus"], false, false), (120, String::new()));
    assert_eq!(run(&["--bogus"], false, true), (2, String::new()));
}

#[test]
fn clap_preserves_forced_colour_and_no_colour() {
    for args in [["--help"], ["--bogus"]] {
        for colour in [true, false] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_drdebits_build"));
            command.args(args).env_remove("CLICOLOR").env_remove("TERM");
            if colour {
                command.env("CLICOLOR_FORCE", "1").env_remove("NO_COLOR");
            } else {
                command.env("NO_COLOR", "1").env_remove("CLICOLOR_FORCE");
            }
            let result = command.output().unwrap();
            let text = if args == ["--help"] {
                result.stdout
            } else {
                result.stderr
            };
            assert_eq!(text.contains(&0x1b), colour, "args={args:?}");
            assert_eq!(
                result.status.code(),
                Some(if args == ["--help"] { 0 } else { 2 })
            );
        }
    }
}
