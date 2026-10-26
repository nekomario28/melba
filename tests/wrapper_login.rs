//! `wrapper login` must not take the Apple ID password as an argument, and
//! must not spawn the wrapper (which would put `-L user:pass` on argv)
//! when argv-scrub.so is missing.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "melbad-wrapper-login-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn saved_binding(&self) -> (PathBuf, Vec<u8>) {
        use sha2::{Digest, Sha256};
        let config = self.0.join("config/melba");
        fs::create_dir_all(&config).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o700)).unwrap();
        let token = b"fixture-existing-token";
        let hash = Sha256::digest(token)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let proof = config.join("wrapper-auth.json");
        let bytes = format!(r#"{{"version":1,"token_sha256":"{hash}"}}"#).into_bytes();
        fs::write(config.join("music-user-token"), token).unwrap();
        fs::write(&proof, &bytes).unwrap();
        for file in [&proof, &config.join("music-user-token")] {
            fs::set_permissions(file, fs::Permissions::from_mode(0o600)).unwrap();
        }
        (proof, bytes)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn password_argument_is_rejected_without_echoing_it() {
    let secret = "fixture-password-do-not-print";
    let output = Command::new(env!("CARGO_BIN_EXE_melbad"))
        .args(["wrapper", "login", "user@example.com", secret])
        .env("MELBA_WRAPPER_PASSWORD", secret)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8(output.stderr).unwrap();
    assert!(err.contains("not accepted as an argument"), "{err}");
    assert!(!err.contains(secret), "rejected argument was echoed: {err}");
    assert!(output.stdout.is_empty());

    let dashed = "-fixture-password-do-not-print";
    let output = Command::new(env!("CARGO_BIN_EXE_melbad"))
        .args(["wrapper", "login", "user@example.com", dashed])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8(output.stderr).unwrap();
    assert!(err.contains("not accepted as an argument"), "{err}");
    assert!(!err.contains(dashed), "rejected argument was echoed: {err}");
}

#[test]
fn login_help_does_not_offer_a_password_argument() {
    let output = Command::new(env!("CARGO_BIN_EXE_melbad"))
        .args(["wrapper", "login", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("MELBA_WRAPPER_PASSWORD"), "{text}");
    assert!(!text.contains("[PASS]"), "{text}");
    assert!(!text.contains("<PASS>"), "{text}");
}

#[test]
fn rejected_login_preflight_preserves_a_saved_binding() {
    for password in ["   ", "fixture-password-with-no-wrapper"] {
        let sandbox = Sandbox::new();
        let (proof, before) = sandbox.saved_binding();
        let output = Command::new(env!("CARGO_BIN_EXE_melbad"))
            .args(["wrapper", "login", "user@example.invalid"])
            .env("HOME", &sandbox.0)
            .env("XDG_DATA_HOME", sandbox.0.join("data"))
            .env("XDG_CONFIG_HOME", sandbox.0.join("config"))
            .env("XDG_CACHE_HOME", sandbox.0.join("cache"))
            .env("MELBA_WRAPPER_PASSWORD", password)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            fs::read(proof).unwrap(),
            before,
            "a login that cannot start invalidated the existing binding"
        );
    }
}

#[test]
fn spawn_failure_preserves_a_saved_binding() {
    for executable in [false, true] {
        let sandbox = Sandbox::new();
        let (proof, before) = sandbox.saved_binding();
        let wrap = sandbox.0.join("data/melba/wrapper");
        let lib = wrap.join("rootfs/system/lib64");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("argv-scrub.so"), b"melba-login-fd-v2").unwrap();
        let run = wrap.join("run");
        fs::write(&run, "#!/no-such-melba-test-interpreter\n").unwrap();
        fs::set_permissions(
            &run,
            fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_melbad"))
            .args(["wrapper", "login", "user@example.invalid"])
            .env("HOME", &sandbox.0)
            .env("XDG_DATA_HOME", sandbox.0.join("data"))
            .env("XDG_CONFIG_HOME", sandbox.0.join("config"))
            .env("XDG_CACHE_HOME", sandbox.0.join("cache"))
            .env("MELBA_WRAPPER_PASSWORD", "fixture-password")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("spawn"));
        assert_eq!(
            fs::read(proof).unwrap(),
            before,
            "a process that never started invalidated the saved binding"
        );
    }
}

#[test]
fn missing_argv_scrub_does_not_spawn_the_wrapper() {
    let sandbox = Sandbox::new();
    let (proof, before) = sandbox.saved_binding();
    let wrap = sandbox.0.join("data/melba/wrapper");
    fs::create_dir_all(&wrap).unwrap();
    let run = wrap.join("run");
    fs::write(&run, "#!/bin/sh\ntouch spawned\n").unwrap();
    fs::set_permissions(&run, fs::Permissions::from_mode(0o755)).unwrap();

    let secret = "fixture-password-do-not-print";
    let output = Command::new(env!("CARGO_BIN_EXE_melbad"))
        .args(["wrapper", "login", "user@example.com"])
        .current_dir(&sandbox.0)
        .env("XDG_DATA_HOME", sandbox.0.join("data"))
        .env("XDG_CONFIG_HOME", sandbox.0.join("config"))
        .env("MELBA_WRAPPER_PASSWORD", secret)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let err = String::from_utf8(output.stderr).unwrap();
    assert!(err.contains("argv-scrub.so"), "{err}");
    assert!(
        !err.contains(secret),
        "password leaked into the error: {err}"
    );
    assert!(
        !wrap.join("spawned").exists(),
        "wrapper was spawned without argv-scrub.so"
    );
    assert_eq!(
        fs::read(proof).unwrap(),
        before,
        "an unavailable credential transport invalidated the existing binding"
    );
}
