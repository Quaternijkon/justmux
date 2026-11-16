//! Regression coverage for the two installed command names.
//!
//! Cargo exposes each binary target through `CARGO_BIN_EXE_*` while running
//! integration tests. Running both entry points here catches accidental
//! removal or divergence of the `jm` and `justmux` install targets without
//! starting a tmux server or opening a terminal.

use std::process::Command;

#[test]
fn both_installed_entry_points_report_the_same_version() {
    for (name, variable) in [
        ("jm", "CARGO_BIN_EXE_jm"),
        ("justmux", "CARGO_BIN_EXE_justmux"),
    ] {
        let executable = std::env::var_os(variable)
            .unwrap_or_else(|| panic!("Cargo did not provide {variable} for {name}"));
        let output = Command::new(executable)
            .arg("--version")
            .output()
            .unwrap_or_else(|error| panic!("could not run {name}: {error}"));
        assert!(
            output.status.success(),
            "{name} --version failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .ends_with(env!("CARGO_PKG_VERSION")),
            "{name} must report the package version: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}
