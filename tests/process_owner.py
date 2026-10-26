#!/usr/bin/env python3
"""Isolated process-lifetime/lock regression; no daemon, SDK, bus or network."""
import fcntl
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time


def alive(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] != "Z"
    except FileNotFoundError:
        return False


def service(folder, escape=False):
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    grandchild = os.fork()
    if grandchild == 0:
        if escape:
            # Leaves the worker's process group: a group signal cannot reach
            # it, so only the supervisor's own descendant cleanup can.
            os.setsid()
        while True:
            signal.pause()
    (folder / "pids").write_text(f"{os.getpid()} {grandchild}")
    while True:
        signal.pause()


def parent(helper, folder, threaded, escape=False):
    lock = (folder / "lock").open("w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    service_args = [sys.executable, __file__, "--service", str(folder)] + (["--escape"] if escape else [])
    def launch():
        args = service_args if os.environ.get("MELBA_OWNER_TEST_DIRECT") == "1" else [helper, str(os.getpid()), *service_args]
        child = subprocess.Popen(args, start_new_session=True, pass_fds=(lock.fileno(),))
        (folder / "supervisor").write_text(str(child.pid))
    if threaded:
        thread = threading.Thread(target=launch)
        thread.start()
        thread.join()
    else:
        launch()
    while True:
        signal.pause()


def exercise(helper, folder, threaded, escape=False):
    parent_process = subprocess.Popen([sys.executable, __file__, "--parent", helper, str(folder), str(int(threaded)), str(int(escape))])
    handles = []
    try:
        deadline = time.monotonic() + 5
        while not ((folder / "pids").exists() and (folder / "supervisor").exists()):
            assert time.monotonic() < deadline, "owned fixture never started"
            time.sleep(0.01)
        owned = set(map(int, (folder / "pids").read_text().split()))
        owned.add(int((folder / "supervisor").read_text()))
        handles = [os.pidfd_open(pid) for pid in owned]
        time.sleep(0.15)
        assert all(alive(pid) for pid in owned), "creating thread exit killed a process-owned service"
        parent_process.kill()
        parent_process.wait(timeout=2)
        deadline = time.monotonic() + 2
        while any(alive(pid) for pid in owned) and time.monotonic() < deadline:
            time.sleep(0.01)
        assert not any(alive(pid) for pid in owned), "owned child survived owner process death"
        with (folder / "lock").open("r+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if escape:
            print("OWNED_ESCAPED_DESCENDANT_DEATH_PASS")
        else:
            print("OWNED_PROCESS_THREAD_LIFETIME_PASS" if threaded else "OWNED_PROCESS_PARENT_DEATH_PASS")
    finally:
        if parent_process.poll() is None:
            parent_process.kill()
            parent_process.wait()
        # Kernel handles prevent cleanup from signaling a recycled/unrelated PID.
        for handle in handles:
            try:
                signal.pidfd_send_signal(handle, signal.SIGKILL)
            except ProcessLookupError:
                pass
            finally:
                os.close(handle)


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--service":
        service(Path(sys.argv[2]), "--escape" in sys.argv[3:])
    elif len(sys.argv) > 1 and sys.argv[1] == "--parent":
        parent(
            sys.argv[2],
            Path(sys.argv[3]),
            bool(int(sys.argv[4])),
            len(sys.argv) > 5 and bool(int(sys.argv[5])),
        )
    else:
        with tempfile.TemporaryDirectory(prefix="melba-owner-") as name:
            for threaded in [False, True]:
                folder = Path(name) / str(int(threaded))
                folder.mkdir()
                exercise(sys.argv[1], folder, threaded)
            folder = Path(name) / "escaped"
            folder.mkdir()
            exercise(sys.argv[1], folder, False, escape=True)


if __name__ == "__main__":
    main()
