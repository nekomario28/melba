"""PTY regression for the real CLI, using only fake credentials and a stub wrapper."""
import errno
import hashlib
import os
from pathlib import Path
import pty
import select
import signal
import subprocess
import sys
import tempfile
import termios
import time
import unittest

BINARY = str(Path(sys.argv.pop(1)).resolve())
PASSWORD = b"fixture password & percent%"
USER = "audit@example.invalid"


class WrapperPasswordTests(unittest.TestCase):
    def setUp(self):
        cache = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache")
        cache.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="melba-password-test-", dir=cache)
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.capture = root / "captured.sha256"
        wrapper = root / "data/melba/wrapper/run"
        wrapper.parent.mkdir(parents=True)
        lib = wrapper.parent / "rootfs/system/lib64"
        lib.mkdir(parents=True)
        source = Path(__file__).resolve().parents[1] / "packaging/argv-scrub.c"
        subprocess.run(["cc", "-O2", "-fPIC", "-shared", "-nostdlib",
                        "-fno-stack-protector", "-fno-builtin", "-Wl,--hash-style=sysv",
                        "-Wl,-z,noexecstack", "-o", str(lib / "argv-scrub.so"), str(source)],
                       check=True, capture_output=True)
        wrapper.write_text('#!/bin/sh\n[ "$1" = "-L" ] || exit 2\n'
                           'printf "%s" "$2" | sha256sum > "$MELBA_AUTH_TEST_CAPTURE"\n'
                           # Stay a login-bearing leaf briefly so the parent can observe the shim.
                           'i=0; while [ "$i" -lt 40000 ]; do i=$((i+1)); done\n'
                           'if [ -n "$MELBA_AUTH_TEST_2FA" ]; then\n'
                           '  i=0\n'
                           '  while [ ! -s "$MELBA_AUTH_TEST_2FA" ] && [ "$i" -lt 100 ]; do\n'
                           '    sleep 0.02; i=$((i+1))\n'
                           '  done\n'
                           'fi\n')
        wrapper.chmod(0o700)
        self.env = dict(os.environ, HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "config"),
                        XDG_DATA_HOME=str(root / "data"), XDG_CACHE_HOME=str(root / "cache"),
                        MELBA_AUTH_TEST_CAPTURE=str(self.capture), MELBA_AUTH_TEST_2FA="")
        self.env.pop("LD_PRELOAD", None)

    def command(self):
        return [BINARY, "wrapper", "login", USER]

    def pty_case(self, typed, expected_exit):
        master, slave = pty.openpty()
        original = termios.tcgetattr(slave)
        self.assertTrue(original[3] & termios.ECHO)
        child = subprocess.Popen(self.command(), stdin=slave, stdout=slave, stderr=slave,
                                 env=self.env, start_new_session=True)
        transcript = bytearray()
        try:
            deadline = time.monotonic() + 8
            sent = False
            while time.monotonic() < deadline:
                if select.select([master], [], [], 0.05)[0]:
                    try:
                        transcript.extend(os.read(master, 65536))
                    except OSError as exc:
                        if exc.errno != errno.EIO:
                            raise
                if not sent and b"Apple ID password for " in transcript:
                    # Wait for hidden-input mode rather than racing the prompt
                    # write against the terminal configuration syscall.
                    if not termios.tcgetattr(slave)[3] & termios.ECHO:
                        os.write(master, typed)
                        sent = True
                if child.poll() is not None:
                    while select.select([master], [], [], 0)[0]:
                        transcript.extend(os.read(master, 65536))
                    break
            self.assertTrue(sent, "CLI never disabled terminal echo before password input")
            child.wait(timeout=1)
            self.assertEqual(child.returncode, expected_exit, transcript.decode(errors="replace"))
            self.assertEqual(termios.tcgetattr(slave), original, "terminal attributes were not restored")
            self.assertNotIn(PASSWORD, transcript, "password appeared in terminal output")
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
            os.close(master)
            os.close(slave)

    def assert_password_reached_wrapper(self):
        expected = hashlib.sha256(USER.encode() + b":" + PASSWORD).hexdigest()
        self.assertEqual(self.capture.read_text().split()[0], expected)

    def test_password_is_hidden_and_reaches_wrapper(self):
        self.pty_case(PASSWORD + b"\n", 0)
        self.assert_password_reached_wrapper()

    def test_ctrl_c_restores_terminal_without_starting_wrapper(self):
        self.pty_case(b"\x03", -signal.SIGINT)
        self.assertFalse(self.capture.exists())

    def test_eof_restores_terminal_without_starting_wrapper(self):
        self.pty_case(b"\x04", 1)
        self.assertFalse(self.capture.exists())

    def assert_redirected_two_lines(self, newline, use_file):
        root = Path(self.temp.name)
        code_path = root / "data/melba/wrapper/rootfs/data/data/com.apple.android.music/files/2fa.txt"
        code_path.parent.mkdir(parents=True, exist_ok=True)
        self.env["MELBA_AUTH_TEST_2FA"] = str(code_path)
        code_path.unlink(missing_ok=True)
        payload = PASSWORD + newline + b"654321" + newline
        if use_file:
            credentials = root / "fake-input"
            credentials.write_bytes(payload)
            with credentials.open("rb") as source:
                result = subprocess.run(self.command(), stdin=source, capture_output=True,
                                        env=self.env, timeout=8)
        else:
            result = subprocess.run(self.command(), input=payload, capture_output=True,
                                    env=self.env, timeout=8)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        self.assert_password_reached_wrapper()
        self.assertTrue(code_path.is_file(), "2FA code was not delivered from the second line")
        self.assertEqual(code_path.read_text(), "654321", "password was reused as the 2FA code")
        self.assertNotIn(PASSWORD, result.stdout + result.stderr)

    def test_redirected_file_does_not_reuse_password_as_2fa(self):
        self.assert_redirected_two_lines(b"\n", use_file=True)

    def test_redirected_crlf_keeps_2fa_on_next_line(self):
        for use_file in (True, False):
            with self.subTest(use_file=use_file):
                self.assert_redirected_two_lines(b"\r\n", use_file=use_file)

    def test_redirected_password_preserves_surrounding_spaces(self):
        password = b"  " + PASSWORD + b"  "
        result = subprocess.run(self.command(), input=password + b"\n", capture_output=True,
                                env=self.env, timeout=8)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        expected = hashlib.sha256(USER.encode() + b":" + password).hexdigest()
        self.assertEqual(self.capture.read_text().split()[0], expected)
        self.assertNotIn(password, result.stdout + result.stderr)

    def test_redirected_stdin_still_works_without_echo(self):
        result = subprocess.run(self.command(), input=PASSWORD + b"\n", capture_output=True,
                                env=self.env, timeout=8)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        self.assertNotIn(PASSWORD, result.stdout + result.stderr)
        self.assert_password_reached_wrapper()


if __name__ == "__main__":
    unittest.main()
