use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "melbad-auth-cli-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn melbad(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_melbad"))
            .args(args)
            .env("HOME", &self.0)
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}/no-bus", self.0.display()),
            )
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("XDG_CACHE_HOME", self.0.join("cache"))
            .output()
            .unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn status_never_prints_the_token_or_its_prefix() {
    let sandbox = Sandbox::new();
    let token = "private-fixture-token-do-not-print";
    assert!(sandbox.melbad(&["auth", "set", token]).status.success());
    let output = sandbox.melbad(&["auth", "status"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("stored"));
    assert!(
        !text.contains(&token[..24]),
        "status leaked a credential prefix"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn wrapper_password_is_hidden_and_terminal_restored() {
    let output = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/wrapper_password.py"
        ))
        .arg(env!("CARGO_BIN_EXE_melbad"))
        .output()
        .expect("Python 3 is required for the CLI's PTY regression tests");
    assert!(
        output.status.success(),
        "PTY regression failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn gui_restore_cannot_complete_from_a_saved_binding_without_native_authentication() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let token = "fixture-previous-session-token";
    assert!(sandbox.melbad(&["auth", "set", token]).status.success());
    let config = sandbox.0.join("config/melba");
    fs::write(config.join("config.json"), r#"{"wrapper_mode":"managed"}"#).unwrap();
    let hash = Sha256::digest(token)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    fs::write(
        config.join(melba_auth::PROOF_FILE),
        format!(r#"{{"version":1,"token_sha256":"{hash}"}}"#),
    )
    .unwrap();
    let wrapper = sandbox.0.join("data/melba/wrapper");
    let file = wrapper.join("rootfs/data/data/com.apple.android.music/files/MUSIC_TOKEN");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, token).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let launcher = wrapper.join("run");
    fs::write(&launcher, "#!/bin/sh\ntouch unexpected-launch\nexit 99\n").unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
    let output = sandbox.melbad(&["auth", "gui", "--restore"]);
    assert!(!output.status.success());
    let event: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(event["type"], "failed");
    assert_eq!(
        fs::read_to_string(config.join("music-user-token")).unwrap(),
        token
    );
    assert!(!wrapper.join("unexpected-launch").exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(token));
}

#[test]
fn invalid_set_fails_without_replacing_an_existing_token() {
    let sandbox = Sandbox::new();
    assert!(sandbox
        .melbad(&["auth", "set", "existing-token"])
        .status
        .success());
    let output = sandbox.melbad(&["auth", "set", "bad\nvalue"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid music-user-token"));
    assert_eq!(
        fs::read_to_string(sandbox.0.join("config/melba/music-user-token")).unwrap(),
        "existing-token"
    );
}

/// A failed GUI attempt tombstones the native binding, which keeps the managed
/// wrapper out of use until a valid sign-in completes. Clearing the local
/// sign-in state must release that tombstone, or one failed live attempt would
/// strand playback with no supported way back.
#[test]
fn clearing_sign_in_forgets_a_failed_binding() {
    let sandbox = Sandbox::new();
    let config = sandbox.0.join("config/melba");
    let proof = config.join("wrapper-auth.json");
    fs::create_dir_all(&config).unwrap();
    assert!(sandbox
        .melbad(&["auth", "set", "kept-token"])
        .status
        .success());
    fs::write(&proof, r#"{"version":1,"token_sha256":""}"#).unwrap();

    let blocked = sandbox.melbad(&["wrapper", "start"]);
    assert!(!blocked.status.success());
    assert!(
        String::from_utf8_lossy(&blocked.stderr).contains("complete GUI sign-in"),
        "a present-but-invalid binding did not block the managed wrapper"
    );

    assert!(sandbox.melbad(&["auth", "clear"]).status.success());
    assert!(!proof.exists(), "auth clear kept the failed binding");
    assert!(!config.join("music-user-token").exists());

    let released = sandbox.melbad(&["wrapper", "start"]);
    assert!(
        !String::from_utf8_lossy(&released.stderr).contains("complete GUI sign-in"),
        "the cleared binding still blocks the managed wrapper"
    );
}
