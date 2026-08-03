use virtui::infrastructure::process::{
    CliRunner, InteractiveRunner, ProcessError, ProcessSpec, TokioCliRunner,
};

#[tokio::test]
async fn a_zero_exit_succeeds_and_preserves_both_streams_untrimmed() {
    let runner = TokioCliRunner;

    let output = runner
        .run(ProcessSpec::new(
            "/bin/sh",
            &["-c", "printf 'local\\n'; printf 'note\\n' >&2"],
        ))
        .await
        .expect("exit status 0 is the only successful result");

    assert_eq!(output.stdout, "local\n");
    assert_eq!(output.stderr, "note\n");
}

#[tokio::test]
async fn an_absent_executable_is_distinct_from_a_process_that_ran() {
    let runner = TokioCliRunner;

    let error = runner
        .run(ProcessSpec::new("virtui-no-such-provider-cli", &["list"]))
        .await
        .expect_err("the program does not exist");

    assert_eq!(error, ProcessError::ExecutableNotFound);
}

#[tokio::test]
async fn a_program_that_cannot_be_spawned_is_distinct_from_an_absent_one() {
    let runner = TokioCliRunner;

    let error = runner
        .run(ProcessSpec::new("/bin", &[]))
        .await
        .expect_err("a directory exists but cannot be executed");

    let ProcessError::SpawnFailed(reason) = error else {
        panic!("expected a spawn failure, got {error:?}");
    };
    assert!(!reason.is_empty(), "the OS reason is preserved");
}

#[tokio::test]
async fn a_signalled_process_reports_a_failure_without_an_exit_code() {
    let runner = TokioCliRunner;

    let error = runner
        .run(ProcessSpec::new("/bin/sh", &["-c", "kill -9 $$"]))
        .await
        .expect_err("a signalled process never succeeds");

    let ProcessError::Exited(failure) = error else {
        panic!("expected a completed process that reported failure, got {error:?}");
    };
    assert_eq!(failure.exit_code, None);
}

#[tokio::test]
async fn a_non_zero_exit_is_a_failure_that_preserves_status_and_output() {
    let runner = TokioCliRunner;

    let error = runner
        .run(ProcessSpec::new(
            "/bin/sh",
            &["-c", "printf listing; printf denied >&2; exit 3"],
        ))
        .await
        .expect_err("a non-zero exit is never a success");

    let ProcessError::Exited(failure) = error else {
        panic!("expected a completed process that reported failure, got {error:?}");
    };
    assert_eq!(failure.exit_code, Some(3));
    assert_eq!(failure.stdout, "listing");
    assert_eq!(failure.stderr, "denied");
}

/// An Interactive Shell that ran and left cleanly is the ordinary case, and the
/// runner has nothing to hand back: everything the process had to say went to
/// the terminal the user was already looking at.
#[test]
fn an_interactive_process_that_exits_cleanly_succeeds() {
    let runner = TokioCliRunner;

    runner
        .run_interactive(&ProcessSpec::new("/bin/sh", &["-c", "exit 0"]))
        .expect("exit status 0 is the only successful result");
}

/// The streams belong to the user, not to Virtui. A process that printed is
/// still quoted nowhere, because nothing was captured to quote — which is why
/// the status is the only thing a failure can carry.
///
/// The two lines this leaks past the test harness are the evidence: they went
/// to the real terminal, which is exactly where an Interactive Shell's output
/// is supposed to go.
#[test]
fn an_interactive_process_keeps_its_status_and_captures_neither_stream() {
    let runner = TokioCliRunner;

    let error = runner
        .run_interactive(&ProcessSpec::new(
            "/bin/sh",
            &[
                "-c",
                "printf 'virtui test: inherited stdout, not captured\\n'; \
                 printf 'virtui test: inherited stderr, not captured\\n' >&2; \
                 exit 3",
            ],
        ))
        .expect_err("a non-zero exit is never a success");

    let ProcessError::Exited(failure) = error else {
        panic!("expected a completed process that reported failure, got {error:?}");
    };
    assert_eq!(failure.exit_code, Some(3));
    assert!(
        failure.stdout.is_empty() && failure.stderr.is_empty(),
        "an inherited stream leaves nothing behind, got {failure:?}"
    );
}

/// A signalled shell — the user pressing Ctrl-C hard enough to kill it — ran,
/// so it is a completed process with no status of its own rather than one that
/// never started.
#[test]
fn a_signalled_interactive_process_ran_and_reports_no_exit_code() {
    let runner = TokioCliRunner;

    let error = runner
        .run_interactive(&ProcessSpec::new("/bin/sh", &["-c", "kill -9 $$"]))
        .expect_err("a signalled process never succeeds");

    let ProcessError::Exited(failure) = error else {
        panic!("expected a completed process that reported failure, got {error:?}");
    };
    assert_eq!(failure.exit_code, None);
}

/// The one failure Virtui does report to the user: the Provider CLI was there
/// at discovery and is gone by the time a shell is asked for.
#[test]
fn an_absent_interactive_program_is_distinct_from_a_process_that_ran() {
    let runner = TokioCliRunner;

    let error = runner
        .run_interactive(&ProcessSpec::new("virtui-no-such-provider-cli", &["exec"]))
        .expect_err("the program does not exist");

    assert_eq!(error, ProcessError::ExecutableNotFound);
}

/// Present but unrunnable is its own failure, because "install it" is the wrong
/// advice for a program that is already there.
#[test]
fn an_interactive_program_that_cannot_be_spawned_is_distinct_from_an_absent_one() {
    let runner = TokioCliRunner;

    let error = runner
        .run_interactive(&ProcessSpec::new("/bin", &[]))
        .expect_err("a directory exists but cannot be executed");

    let ProcessError::SpawnFailed(reason) = error else {
        panic!("expected a spawn failure, got {error:?}");
    };
    assert!(!reason.is_empty(), "the OS reason is preserved");
}
