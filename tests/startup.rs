//! Startup behaviour observed at the only seam that can show it: the process.
//!
//! Whether diagnostics reach the user *before* raw mode, and whether Tuivir
//! exits non-zero, are properties of the program rather than of any library
//! call. Only invalid configurations are run here, so no test ever reaches
//! `ratatui::init` and none needs a terminal.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// Runs Tuivir with `TUIVIR_CONFIG_FILE` pointing at `contents`.
///
/// The explicit override has the highest precedence, so discovery never runs
/// and the test cannot depend on the machine's home directory.
fn start_with_config(name: &str, contents: &str) -> Output {
    let path = std::env::temp_dir().join(format!("tuivir-startup-{name}.toml"));
    std::fs::write(&path, contents).expect("a configuration file to select");
    let output = run(&path);
    let _ = std::fs::remove_file(&path);
    output
}

fn run(config: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tuivir"))
        .env("TUIVIR_CONFIG_FILE", config)
        .output()
        .expect("Tuivir to run")
}

fn run_version() -> Output {
    Command::new(env!("CARGO_BIN_EXE_tuivir"))
        .arg("--version")
        .env("TUIVIR_CONFIG_FILE", "/nonexistent/tuivir/config.toml")
        .output()
        .expect("Tuivir to run")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr to be text")
}

#[test]
fn version_prints_the_package_version_before_startup() {
    let output = run_version();

    assert!(
        output.status.success(),
        "Tuivir --version must exit successfully, got {:?}",
        output.status
    );
    assert_eq!(
        output.stdout,
        format!("{}\n", env!("CARGO_PKG_VERSION")).as_bytes(),
        "--version must print only the package version"
    );
    assert!(
        output.stderr.is_empty(),
        "--version must not emit diagnostics, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A rejected configuration must leave the terminal untouched and say why.
/// Every diagnostic appears in one run, so the file can be fixed in one pass.
#[test]
fn an_invalid_configuration_reports_every_diagnostic_and_exits_non_zero() {
    let output = start_with_config(
        "invalid",
        "[keybindings]\n\
         \"no_such_command\" = [\"x\"]\n\
         \"resource_restart\" = [\"f13\"]\n\
         \"resource_stop\" = [\"j\"]\n",
    );

    assert!(
        !output.status.success(),
        "Tuivir must exit non-zero, got {:?}",
        output.status
    );
    let stderr = stderr(&output);
    for expected in [
        "no_such_command",
        "f13",
        "selection_next",
        "resource_stop",
        "tuivir-startup-invalid.toml",
    ] {
        assert!(
            stderr.contains(expected),
            "stderr should mention {expected:?}, got:\n{stderr}"
        );
    }
    assert!(
        output.stdout.is_empty(),
        "diagnostics belong on stderr, but stdout had: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// The file Tuivir refused is named, so a user running several configurations
/// can tell which one was actually selected.
#[test]
fn an_unparsable_configuration_names_the_file_it_refused() {
    let output = start_with_config("unparsable", "[keybindings\n");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("tuivir-startup-unparsable.toml"),
        "the selected file must be named, got:\n{}",
        stderr(&output)
    );
}

/// A missing explicitly selected file fails loudly rather than falling back to
/// defaults, so a mistyped path is never mistaken for "no configuration".
#[test]
fn a_missing_explicitly_selected_file_exits_non_zero() {
    let path = PathBuf::from("/nonexistent/tuivir/does-not-exist.toml");

    let output = run(&path);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("does-not-exist.toml"),
        "got:\n{}",
        stderr(&output)
    );
}
