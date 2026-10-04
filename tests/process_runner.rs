use tuivir::infrastructure::process::{CliRunner, ProcessError, ProcessSpec, TokioCliRunner};

// This subprocess gives the test below a real parent that can exit, rather
// than merely dropping a future while the same Tokio runtime stays alive.
#[test]
fn background_launch_parent() {
    let Some(marker) = std::env::var_os("TUIVIR_TEST_LAUNCH_MARKER") else {
        return;
    };
    let marker = marker.to_str().unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        TokioCliRunner
            .run(ProcessSpec::background(
                "/bin/sh",
                &["-c", "sleep 2; printf alive > \"$1\"", "test", marker],
            ))
            .await
            .unwrap();
    });
}

#[tokio::test]
async fn a_background_provider_process_survives_its_parent_exiting() {
    let directory = std::env::temp_dir().join(format!("tuivir-parent-exit-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let marker = directory.join("completed");
    let parent = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "background_launch_parent"])
        .env("TUIVIR_TEST_LAUNCH_MARKER", &marker)
        .env("XDG_STATE_HOME", &directory)
        .output()
        .unwrap();
    assert!(
        parent.status.success(),
        "{}",
        String::from_utf8_lossy(&parent.stderr)
    );
    let completed = tokio::time::timeout(std::time::Duration::from_secs(4), async {
        while !marker.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        std::fs::read_to_string(&marker).unwrap()
    })
    .await;
    // This directory contains only the marker and logs from this test's child.
    std::fs::remove_dir_all(&directory).unwrap();
    assert_eq!(
        completed.expect("Provider continues after parent exit"),
        "alive"
    );
}

#[tokio::test]
async fn a_background_process_returns_before_exit_and_keeps_running() {
    let path = std::env::temp_dir().join(format!("tuivir-background-{}.done", std::process::id()));
    let runner = TokioCliRunner;
    runner
        .run(ProcessSpec::background(
            "/bin/sh",
            &[
                "-c",
                "sleep 2; printf done > \"$1\"",
                "test",
                path.to_str().unwrap(),
            ],
        ))
        .await
        .expect("background launch");
    assert!(
        !path.exists(),
        "launch returns before the long-lived process exits"
    );
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        while !path.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the child continues after launch returns");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "done");
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn a_background_launch_preserves_immediate_failure_diagnostics() {
    let error = TokioCliRunner
        .run(ProcessSpec::background(
            "/bin/sh",
            &["-c", "printf 'invalid VM configuration' >&2; exit 3"],
        ))
        .await
        .expect_err("startup failed");
    let ProcessError::Exited(failure) = error else {
        panic!("expected startup failure: {error:?}");
    };
    assert_eq!(failure.exit_code, Some(3));
    assert_eq!(failure.stderr, "invalid VM configuration");
}

#[tokio::test]
async fn an_absent_background_executable_remains_distinct_from_startup_failure() {
    assert_eq!(
        TokioCliRunner
            .run(ProcessSpec::background("tuivir-no-such-provider-cli", &[]))
            .await,
        Err(ProcessError::ExecutableNotFound)
    );
}

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
        .run(ProcessSpec::new("tuivir-no-such-provider-cli", &["list"]))
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
