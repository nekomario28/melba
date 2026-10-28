#![cfg(all(target_os = "linux", feature = "demo"))]

use std::process::Command;

#[test]
fn an_unavailable_display_leaves_a_useful_log_without_a_console() {
    let directory = std::env::temp_dir().join(format!(
        "melba-startup-diagnostics-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir(&directory).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_melba"))
        .args(["--demo", "--demo-shot"])
        .arg(directory.join("unused.png"))
        .arg("--demo-data")
        .arg(&directory)
        .env_remove("RUST_LOG")
        .env("DISPLAY", "")
        .env("WAYLAND_DISPLAY", "missing-display")
        .env("XDG_RUNTIME_DIR", &directory)
        // A binary started by a test must not reach the user's files, and the
        // daemon it starts inherits these.
        .env("HOME", &directory)
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .env("XDG_STATE_HOME", directory.join("state"))
        .env("XDG_CACHE_HOME", directory.join("cache"))
        .env("XDG_DATA_HOME", directory.join("data"))
        .output()
        .unwrap();
    let log = std::fs::read_to_string(directory.join("state/melba.log")).unwrap();
    std::fs::remove_dir_all(directory).unwrap();

    assert!(!output.status.success(), "the unavailable display opened");
    assert!(
        log.contains(&format!("Starting melba {}", env!("CARGO_PKG_VERSION"))),
        "missing startup identity: {log}"
    );
    assert!(
        log.contains("Native window failed:"),
        "missing error: {log}"
    );
}
