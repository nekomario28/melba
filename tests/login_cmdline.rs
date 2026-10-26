//! The Apple ID credential must not appear in any process's
//! `/proc/<pid>/cmdline` while `wrapper login` starts. A separate process
//! polls every cmdline in a tight loop from before spawn until the child
//! exits. No real wrapper binary and no Apple ID are involved: `run` is a
//! shell script that execs a small `mid` (fork + execve, same shape as
//! upstream's launcher) which execs `final`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "melbad-cmdline-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cc(args: &[std::ffi::OsString]) {
    let st = Command::new("cc").args(args).status().expect("cc");
    assert!(st.success(), "cc failed: {st}");
}

fn os(s: &str) -> std::ffi::OsString {
    s.into()
}

fn compile_scrub(out: &Path) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/argv-scrub.c");
    cc(&[
        os("-O2"),
        os("-fPIC"),
        os("-shared"),
        os("-nostdlib"),
        os("-fno-stack-protector"),
        os("-fno-builtin"),
        os("-o"),
        out.as_os_str().to_owned(),
        src.as_os_str().to_owned(),
        os("-Wl,--hash-style=sysv"),
        os("-Wl,-z,noexecstack"),
    ]);
}

fn compile_c(out: &Path, src_text: &str) {
    let src = out.with_extension("c");
    fs::write(&src, src_text).unwrap();
    cc(&[
        os("-O2"),
        os("-o"),
        out.as_os_str().to_owned(),
        src.as_os_str().to_owned(),
    ]);
}

struct Poll {
    child: std::process::Child,
    result: PathBuf,
    stop: PathBuf,
}

impl Poll {
    fn start(dir: &Path, needle_file: &Path) -> Self {
        let poller = dir.join("poller");
        let result = dir.join("poll-result");
        let stop = dir.join("poll-stop");
        let ready = dir.join("poll-ready");
        let _ = fs::remove_file(&stop);
        let _ = fs::remove_file(&ready);
        compile_c(&poller, include_str!("cmdline_poller.c"));
        let child = Command::new(&poller)
            .arg(needle_file)
            .arg(&result)
            .arg(&stop)
            .arg(&ready)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let t0 = Instant::now();
        while !ready.exists() {
            assert!(
                t0.elapsed() < Duration::from_secs(5),
                "poller did not start"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        Self {
            child,
            result,
            stop,
        }
    }

    fn finish(mut self) -> (i64, i64, i64, i64) {
        fs::write(&self.stop, b"1").unwrap();
        let st = self.child.wait().unwrap();
        assert!(st.success(), "poller {st}");
        let text = fs::read_to_string(&self.result).unwrap();
        let mut samples = 0;
        let mut hits = 0;
        let mut saw_ph = 0;
        let mut saw_applied = 0;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("samples=") {
                samples = v.parse().unwrap();
            } else if let Some(v) = line.strip_prefix("hits=") {
                hits = v.parse().unwrap();
            } else if let Some(v) = line.strip_prefix("saw_placeholder=") {
                saw_ph = v.parse().unwrap();
            } else if let Some(v) = line.strip_prefix("saw_applied=") {
                saw_applied = v.parse().unwrap();
            }
        }
        (samples, hits, saw_ph, saw_applied)
    }
}

fn layout(root: &Sandbox, scrub_src: Option<&str>, sleep_secs: &str) -> PathBuf {
    let wrap = root.path("data/melba/wrapper");
    let lib64 = wrap.join("rootfs/system/lib64");
    fs::create_dir_all(&lib64).unwrap();
    if let Some(src) = scrub_src {
        let c = root.path("inert.c");
        fs::write(&c, src).unwrap();
        cc(&[
            os("-O2"),
            os("-fPIC"),
            os("-shared"),
            os("-nostdlib"),
            os("-fno-stack-protector"),
            os("-fno-builtin"),
            os("-o"),
            lib64.join("argv-scrub.so").as_os_str().to_owned(),
            c.as_os_str().to_owned(),
            os("-Wl,--hash-style=sysv"),
            os("-Wl,-z,noexecstack"),
        ]);
    } else {
        compile_scrub(&lib64.join("argv-scrub.so"));
    }
    compile_c(
        &wrap.join("mid"),
        r#"
#include <unistd.h>
#include <sys/wait.h>
extern char **environ;
int main(int argc, char **argv) {
    pid_t p; int st;
    if (argc < 2) return 2;
    p = fork();
    if (p < 0) return 127;
    if (p > 0) {
        if (waitpid(p, &st, 0) < 0) return 127;
        return WIFEXITED(st) ? WEXITSTATUS(st) : 1;
    }
    execve(argv[1], &argv[1], environ);
    return 127;
}
"#,
    );
    compile_c(
        &wrap.join("final"),
        &format!(
            r#"
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
int main(int argc, char **argv) {{
    FILE *f, *c; int ch;
    if (argc < 2) return 2;
    f = fopen(argv[1], "w");
    if (!f) return 2;
    for (int i = 0; i < argc; i++) fprintf(f, "argv=%s\n", argv[i]);
    fprintf(f, "cmdline=");
    c = fopen("/proc/self/cmdline", "rb");
    if (c) {{ while ((ch = fgetc(c)) != EOF) fputc(ch == 0 ? '|' : ch, f); fclose(c); }}
    fputc('\n', f);
    const char *applied = getenv("MELBA_LOGIN_APPLIED");
    fprintf(f, "environ=MELBA_LOGIN_APPLIED=%s\n", applied ? applied : "");
    fclose(f);
    sleep({sleep_secs});
    return 0;
}}
"#
        ),
    );
    let run = wrap.join("run");
    fs::write(
        &run,
        "#!/bin/sh\nD=\"$(cd \"$(dirname \"$0\")\" && pwd)\"\nexec \"$D/mid\" \"$D/final\" \"$D/argv-out\" \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&run, fs::Permissions::from_mode(0o755)).unwrap();
    wrap
}

fn read_poll_numbers(text: &str) -> (i64, i64, i64, i64) {
    let mut samples = 0;
    let mut hits = 0;
    let mut saw_ph = 0;
    let mut saw_applied = 0;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("samples=") {
            samples = v.parse().unwrap();
        } else if let Some(v) = line.strip_prefix("hits=") {
            hits = v.parse().unwrap();
        } else if let Some(v) = line.strip_prefix("saw_placeholder=") {
            saw_ph = v.parse().unwrap();
        } else if let Some(v) = line.strip_prefix("saw_applied=") {
            saw_applied = v.parse().unwrap();
        }
    }
    (samples, hits, saw_ph, saw_applied)
}

#[test]
fn poller_sees_a_secret_that_is_really_on_argv() {
    let root = Sandbox::new();
    let secret = "user@example.com:fixture-SECRET-control";
    let needle = root.path("needle");
    fs::write(&needle, secret).unwrap();
    compile_c(
        &root.path("sleeper"),
        "#include <unistd.h>\nint main(void) { sleep(2); return 0; }\n",
    );
    let poller = root.path("poller");
    compile_c(&poller, include_str!("cmdline_poller.c"));
    let result = root.path("poll-result");
    let stop = root.path("poll-stop");
    let ready = root.path("poll-ready");
    let mut poll = Command::new(&poller)
        .arg(&needle)
        .arg(&result)
        .arg(&stop)
        .arg(&ready)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let t0 = Instant::now();
    while !ready.exists() {
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "poller did not start"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut child = Command::new(root.path("sleeper"))
        .arg(secret)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let _ = child.kill();
    let _ = child.wait();
    fs::write(&stop, b"1").unwrap();
    assert!(poll.wait().unwrap().success());
    let (samples, hits, _, _) = read_poll_numbers(&fs::read_to_string(&result).unwrap());
    eprintln!("control cmdline poll: samples={samples} hits={hits}");
    assert!(samples >= 1, "samples={samples}");
    assert!(
        hits >= 1,
        "poller missed a cmdline that really contains the credential (samples={samples})"
    );
}

#[test]
fn login_cmdline_never_contains_the_credential() {
    let root = Sandbox::new();
    let user = "user@example.com";
    let pass = "fixture-SECRET-9f3a";
    let cred = format!("{user}:{pass}");
    let _wrap = layout(&root, None, "1");
    let needle = root.path("needle");
    fs::write(&needle, &cred).unwrap();
    let poll = Poll::start(&root.0, &needle);
    let output = Command::new(env!("CARGO_BIN_EXE_melbad"))
        .args(["wrapper", "login", user])
        .env("XDG_DATA_HOME", root.path("data"))
        .env("XDG_CONFIG_HOME", root.path("config"))
        .env("HOME", &root.0)
        .env("MELBA_WRAPPER_PASSWORD", pass)
        .env_remove("LD_PRELOAD")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let (samples, hits, saw_ph, saw_applied) = poll.finish();
    eprintln!(
        "login cmdline poll: samples={samples} hits={hits} saw_placeholder={saw_ph} saw_applied={saw_applied}"
    );
    let err = String::from_utf8_lossy(&output.stderr);
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(
        !err.contains(pass) && !out.contains(pass),
        "credential leaked into melbad output"
    );
    assert!(output.status.success(), "stdout={out} stderr={err}");
    assert!(samples >= 20, "poller barely ran: samples={samples}");
    assert_eq!(
        hits, 0,
        "credential was visible in cmdline (samples={samples})"
    );
    assert!(saw_ph > 0, "never observed the -L @ placeholder");
    assert!(
        saw_applied > 0,
        "shim never confirmed MELBA_LOGIN_APPLIED=1"
    );
    let dumped = fs::read_to_string(root.path("data/melba/wrapper/argv-out")).unwrap();
    let cmdline = dumped.lines().find(|l| l.starts_with("cmdline=")).unwrap();
    assert!(
        !cmdline.contains(&cred),
        "final's own cmdline held the credential"
    );
    assert!(cmdline.contains("|-L|@"), "final cmdline: {cmdline}");
    assert!(
        dumped.lines().any(|l| l == format!("argv={cred}")),
        "final's argv did not receive the credential from the memfd"
    );
    assert!(dumped.contains("MELBA_LOGIN_APPLIED=1"));
}

#[test]
fn login_rejects_confirmation_from_an_earlier_image() {
    for fork_final in [false, true] {
        let root = Sandbox::new();
        let user = "user@example.com";
        let pass = "fixture-SECRET-final-image";
        let cred = format!("{user}:{pass}");
        let wrap = layout(&root, None, "1");
        compile_c(
            &wrap.join("mid"),
            r#"
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/wait.h>
extern char **environ;
int main(int argc, char **argv) {
    if (argc < 3) return 2;
    char witness[4096];
    snprintf(witness, sizeof(witness), "%s.mid", argv[2]);
    FILE *f = fopen(witness, "w");
    if (!f) return 2;
    const char *applied = getenv("MELBA_LOGIN_APPLIED");
    fprintf(f, "%s", applied ? applied : "");
    fclose(f);
    usleep(350000); /* Let the parent observe this confirmed, login-bearing leaf. */
    const char *fork_final = getenv("MELBA_TEST_FORK_FINAL");
    if (fork_final && fork_final[0] == '1') {
        pid_t p = fork();
        if (p < 0) return 127;
        if (p > 0) {
            int status;
            if (waitpid(p, &status, 0) < 0) return 127;
            return WIFEXITED(status) ? WEXITSTATUS(status) : 1;
        }
    }
    unsetenv("LD_PRELOAD");
    execve(argv[1], &argv[1], environ); /* Interposer strips the credential and resets APPLIED. */
    return 127;
}
"#,
        );
        let needle = root.path("needle");
        fs::write(&needle, &cred).unwrap();
        let poll = Poll::start(&root.0, &needle);
        let output = Command::new(env!("CARGO_BIN_EXE_melbad"))
            .args(["wrapper", "login", user])
            .env("XDG_DATA_HOME", root.path("data"))
            .env("XDG_CONFIG_HOME", root.path("config"))
            .env("HOME", &root.0)
            .env("MELBA_WRAPPER_PASSWORD", pass)
            .env("MELBA_TEST_FORK_FINAL", if fork_final { "1" } else { "0" })
            .env_remove("LD_PRELOAD")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let (samples, hits, _, observed_applied) = poll.finish();
        let dumped = fs::read_to_string(wrap.join("argv-out")).unwrap();
        assert_eq!(fs::read_to_string(wrap.join("argv-out.mid")).unwrap(), "1");
        assert!(
            observed_applied > 0,
            "the confirmed intermediate image was never observed"
        );
        assert!(dumped.contains("environ=MELBA_LOGIN_APPLIED=0"), "{dumped}");
        assert!(dumped.lines().any(|line| line == "argv=@"));
        assert!(!dumped.lines().any(|line| line == format!("argv={cred}")));
        assert_eq!(hits, 0, "credential appeared on argv in {samples} samples");
        assert!(
            !output.status.success(),
            "an earlier image's confirmation incorrectly blessed an unconfirmed final image"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("did not take effect"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(pass));
        assert!(!String::from_utf8_lossy(&output.stdout).contains(pass));
    }
}

#[test]
fn login_aborts_when_the_shim_does_not_apply() {
    let root = Sandbox::new();
    let user = "user@example.com";
    let pass = "fixture-SECRET-broken";
    let cred = format!("{user}:{pass}");
    let _wrap = layout(
        &root,
        Some(
            "static const char melba_login_fd_marker[] __attribute__((used)) = \"melba-login-fd-v2\";\n\
             __attribute__((visibility(\"default\"))) int melba_scrub_inert(void) { return 1; }\n",
        ),
        "4",
    );
    let needle = root.path("needle");
    fs::write(&needle, &cred).unwrap();
    let poll = Poll::start(&root.0, &needle);
    let t0 = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_melbad"))
        .args(["wrapper", "login", user])
        .env("XDG_DATA_HOME", root.path("data"))
        .env("XDG_CONFIG_HOME", root.path("config"))
        .env("HOME", &root.0)
        .env("MELBA_WRAPPER_PASSWORD", pass)
        .env_remove("LD_PRELOAD")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let elapsed = t0.elapsed();
    let (samples, hits, _, _) = poll.finish();
    eprintln!(
        "inert-shim cmdline poll: samples={samples} hits={hits} elapsed_ms={}",
        elapsed.as_millis()
    );
    let err = String::from_utf8_lossy(&output.stderr);
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(!err.contains(pass) && !out.contains(pass));
    assert!(
        !output.status.success(),
        "login succeeded without a working shim: {out} {err}"
    );
    assert!(
        err.contains("did not take effect") && err.contains("killed"),
        "stderr={err}"
    );
    assert_eq!(
        hits, 0,
        "credential was visible in cmdline (samples={samples})"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "login was not aborted promptly: {elapsed:?}"
    );
}
