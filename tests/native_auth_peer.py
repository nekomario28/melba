#!/usr/bin/env python3
"""Run the real Rust coordinator and embedded bridge against an isolated fake SDK.

Only the SDK edge and Apple token validator are fake. No Apple code is loaded,
no real profile/bus is used, and no network requests or messages are sent.
"""
import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
TEST = "auth_gui::native::tests::isolated_native_peer_round_trip"
TOKEN = b"fixture-native-music-token"
OLD = b"fixture-previous-music-token"
USER = "audit@example.invalid"
PASSWORD = "fixture-password"


def running(pid):
    try:
        state = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
        return state != "Z"
    except FileNotFoundError:
        return False


def exercise(binary, folder, scenario):
    home = folder / scenario
    wrapper = home / "data/melba/wrapper"
    lib = wrapper / "rootfs/system/lib64"
    bin_dir = wrapper / "rootfs/system/bin"
    lib.mkdir(parents=True)
    bin_dir.mkdir(parents=True)
    subprocess.run(["cc", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror", "-o", str(lib / "libfakesdk.so"), str(ROOT / "tests/fixtures/wrapper_auth_sdk.c")], check=True, timeout=20)
    subprocess.run(["cc", "-Wall", "-Wextra", "-Werror", "-o", str(bin_dir / "main"), str(ROOT / "tests/fixtures/wrapper_auth_main.c"), "-L" + str(lib), "-lfakesdk", "-Wl,-rpath,$ORIGIN/../lib64"], check=True, timeout=20)
    launcher = wrapper / "run"
    launcher.write_text('#!/bin/sh\nD="$(dirname "$0")"\nexec "$D/rootfs/system/bin/main" "$@"\n')
    launcher.chmod(0o700)
    config = home / "config/melba"
    config.mkdir(parents=True)
    (config / "config.json").write_text('{"wrapper_mode":"managed"}')
    saved = config / "music-user-token"
    saved.write_bytes(OLD)
    saved.chmod(0o600)
    native_token = wrapper / "rootfs/data/data/com.apple.android.music/files/MUSIC_TOKEN"
    native_token.parent.mkdir(parents=True)
    native_token.write_bytes(OLD if scenario == "resume" else b"fixture-old-native-token")
    native_token.chmod(0o600)
    if scenario == "resume":
        proof = config / "wrapper-auth.json"
        proof.write_text(json.dumps({"version": 1, "token_sha256": hashlib.sha256(OLD).hexdigest()}))
        proof.chmod(0o600)
    trace, pids, validation = [home / n for n in ["trace", "pids", "validation"]]
    env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home / "config"), XDG_DATA_HOME=str(home / "data"), XDG_CACHE_HOME=str(home / "cache"), DBUS_SESSION_BUS_ADDRESS="unix:path=" + str(home / "no-session-bus"), HTTP_PROXY="http://127.0.0.1:9", HTTPS_PROXY="http://127.0.0.1:9", ALL_PROXY="http://127.0.0.1:9", NO_PROXY="", MELBA_NATIVE_TEST_CHILD="1", FAKE_SCENARIO=scenario, FAKE_TRACE=str(trace), FAKE_PIDS=str(pids), FAKE_TOKEN_FILE=str(native_token), FAKE_VALIDATION_TRACE=str(validation))
    child = subprocess.Popen([binary, "--exact", TEST, "--nocapture"], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    output = bytearray()
    pending = bytearray()
    events = []
    code_count = 0
    deadline = time.monotonic() + 15
    def send(value):
        child.stdin.write(json.dumps(value).encode() + b"\n")
        child.stdin.flush()
    try:
        while time.monotonic() < deadline:
            ready, _, _ = select.select([child.stdout], [], [], 0.2)
            if not ready:
                continue
            chunk = os.read(child.stdout.fileno(), 4096)
            if not chunk:
                break
            output.extend(chunk)
            pending.extend(chunk)
            while b"\n" in pending:
                line, _, pending = pending.partition(b"\n")
                index = line.find(b'{"type":')
                if index < 0:
                    continue  # Rust's test-harness framing, not protocol output.
                event = json.loads(line[index:])
                events.append(event)
                if event["type"] == "prompt":
                    prompt = event["prompt"]
                    if scenario == "cancel":
                        send({"type": "cancel"})
                    elif prompt["kind"] == "dialog":
                        assert [(b["id"], b["label"]) for b in prompt["buttons"]] == [(0, "Cancel"), (1, "Text ••12"), (2, "Call ••12")]
                        send({"type": "select", "id": prompt["id"], "button": 2 if scenario == "voice" else 1})
                    elif prompt["kind"] == "credentials":
                        send({"type": "credentials", "id": prompt["id"], "username": USER, "password": "rejected-password" if scenario == "wrong-password" else PASSWORD})
                    else:
                        code_count += 1
                        code = "111111" if scenario == "wrong-code" and code_count == 1 else "654321"
                        value = {"type": "code", "id": prompt["id"], "code": code}
                        if prompt["kind"] == "credentials_and_code":
                            value.update(type="credentials_and_code", username=USER, password=PASSWORD)
                        send(value)
                elif event["type"] == "ready_to_commit":
                    if scenario == "peer-loss":
                        child.kill()
                        continue
                    send({"type": "cancel" if scenario == "cancel-commit" else "commit"})
        else:
            raise AssertionError("native fixture timed out")
        child.stdin.close()
        child.wait(timeout=5)
        errors = child.stderr.read()
        assert child.returncode == (-9 if scenario == "peer-loss" else 0), (output.decode(errors="replace"), errors.decode(errors="replace"))
        assert USER.encode() not in output and PASSWORD.encode() not in output and b"654321" not in output
        success = scenario in {"sms", "voice", "wrong-code", "cached-code", "device", "resume"}
        assert any(e["type"] == "complete" for e in events) == success, events
        assert saved.read_bytes() == (TOKEN if success else OLD)
        if success:
            assert validation.read_text() == "validator-called"
            proof = json.loads((config / "wrapper-auth.json").read_text())
            assert proof == {"version": 1, "token_sha256": hashlib.sha256(TOKEN).hexdigest()}
            assert saved.stat().st_mode & 0o777 == 0o600
            assert (config / "wrapper-auth.json").stat().st_mode & 0o777 == 0o600
            if scenario not in {"device", "resume"}:
                assert ("voice-request-returned" if scenario == "voice" else "sms-request-returned") in trace.read_text()
            if scenario == "wrong-code": assert code_count == 2
            if scenario == "cached-code": assert any(e.get("prompt", {}).get("kind") == "credentials_and_code" for e in events)
        elif scenario in {"cancel", "native-rejected", "stale-token", "zero-exit", "wrong-password", "delivery-rejected"}:
            assert not validation.exists(), "an old/non-authenticated token reached validation"
        if trace.exists():
            text = trace.read_text()
            assert text.count("sdk-response-retained") == text.count("sdk-response-released")
        assert not list(wrapper.rglob("2fa.txt"))
        if pids.exists():
            owned = [int(x) for x in pids.read_text().split()]
            end = time.monotonic() + 2
            while any(running(pid) for pid in owned) and time.monotonic() < end:
                time.sleep(0.02)
            assert not any(running(pid) for pid in owned), "owned native process survived cancellation/completion"
            assert not Path(f"/proc/{owned[0]}").exists(), "direct child was not reaped"
        print("NATIVE_FAKE_PEER_PASS", scenario)
    finally:
        if child.poll() is None:
            child.stdin.close()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()


def main():
    cache = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "melba-native-auth-tests"
    cache.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=cache) as name:
        cases = ["sms", "voice", "device", "wrong-code", "cached-code", "resume", "wrong-password", "native-rejected", "delivery-rejected", "stale-token", "zero-exit", "validation-rejected", "cancel", "cancel-commit", "peer-loss", "restore-failed"]
        selected = os.environ.get("MELBA_NATIVE_TEST_CASES", "").split(",") if os.environ.get("MELBA_NATIVE_TEST_CASES") else cases
        assert set(selected) <= set(cases), "unknown fixture case"
        for scenario in selected:
            exercise(sys.argv[1], Path(name), scenario)


if __name__ == "__main__":
    main()
