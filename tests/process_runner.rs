use virtui::cli::{CliRunner, ProcessError, ProcessSpec, TokioCliRunner};

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
