//! Startup behaviour observed at the only seam that can show it: the process.
//!
//! Whether diagnostics reach the user *before* raw mode, and whether Virtui
//! exits non-zero, are properties of the program rather than of any library
//! call. Only invalid configurations are run here, so no test ever reaches
//! `ratatui::init` and none needs a terminal.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// Runs Virtui with `VIRTUI_CONFIG_FILE` pointing at `contents`.
///
/// The explicit override has the highest precedence, so discovery never runs
/// and the test cannot depend on the machine's home directory.
fn start_with_config(name: &str, contents: &str) -> Output {
    let path = std::env::temp_dir().join(format!("virtui-startup-{name}.toml"));
    std::fs::write(&path, contents).expect("a configuration file to select");
    let output = run(&path);
    let _ = std::fs::remove_file(&path);
    output
}

fn run(config: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_virtui"))
        .env("VIRTUI_CONFIG_FILE", config)
        .output()
        .expect("Virtui to run")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr to be text")
}

/// A rejected configuration must leave the terminal untouched and say why.
/// Every diagnostic appears in one run, so the file can be fixed in one pass.
#[test]
fn an_invalid_configuration_reports_every_diagnostic_and_exits_non_zero() {
    let output = start_with_config(
        "invalid",
        "[keybindings]\n\
         \"no.such.command\" = [\"x\"]\n\
         \"resource.restart\" = [\"f13\"]\n\
         \"resource.stop\" = [\"j\"]\n",
    );

    assert!(
        !output.status.success(),
        "Virtui must exit non-zero, got {:?}",
        output.status
    );
    let stderr = stderr(&output);
    for expected in [
        "no.such.command",
        "f13",
        "selection.next",
        "resource.stop",
        "virtui-startup-invalid.toml",
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

/// The file Virtui refused is named, so a user running several configurations
/// can tell which one was actually selected.
#[test]
fn an_unparsable_configuration_names_the_file_it_refused() {
    let output = start_with_config("unparsable", "[keybindings\n");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("virtui-startup-unparsable.toml"),
        "the selected file must be named, got:\n{}",
        stderr(&output)
    );
}

/// A missing explicitly selected file fails loudly rather than falling back to
/// defaults, so a mistyped path is never mistaken for "no configuration".
#[test]
fn a_missing_explicitly_selected_file_exits_non_zero() {
    let path = PathBuf::from("/nonexistent/virtui/does-not-exist.toml");

    let output = run(&path);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("does-not-exist.toml"),
        "got:\n{}",
        stderr(&output)
    );
}
