//! The fuzz target's seed corpus replays on every platform, and its checks
//! bite (ported from test_fuzz_harness.py).

use std::path::Path;

use drdebits::error::Fail;
use drdebits::fuzz::{check_input, check_with};
use drdebits::yaml::Value;

#[test]
fn seed_corpus_replays() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz")
        .join("corpus");
    let mut seeds = 0;
    for entry in std::fs::read_dir(&corpus).unwrap() {
        let path = entry.unwrap().path();
        let data = std::fs::read(&path).unwrap();
        let result = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || check_input(&data))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(result, Ok(()), "{}", path.display());
        seeds += 1;
    }
    assert!(seeds >= 10, "the seed corpus is present");
}

#[test]
fn an_unknown_decision_status_is_a_failure() {
    let result = check_with(
        b"x",
        |_| vec!["MAYBE".into()],
        |_| Ok(Value::Dict(Default::default())),
    );
    assert_eq!(
        result.unwrap_err(),
        "decision reader returned unknown status 'MAYBE'"
    );
}

#[test]
fn a_reader_returning_a_non_mapping_is_a_failure() {
    let result = check_with(
        b"- a\n",
        |_| vec![],
        |_| Ok(Value::List(Default::default())),
    );
    assert!(result.unwrap_err().contains("non-mapping"));
}

#[test]
fn unexpected_reader_errors_fail_the_run() {
    let result = check_with(b"a: 1\n", |_| vec![], |_| Err(Fail::Internal("bug".into())));
    assert!(result.unwrap_err().contains("bug"));
}

#[test]
fn a_refusal_is_an_accepted_outcome() {
    assert_eq!(check_input(b"- a\n"), Ok(()));
    assert_eq!(check_input(b"\xff"), Ok(()));
}
