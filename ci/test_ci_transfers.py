"""Every job that runs apt must arm the transfer timeouts first.

Three jobs have hung on a mirror that stopped sending bytes: release-install-smoke
twice (killed by its own limit) and gates once (44 minutes, no limit of its own,
holding up a release). The timeout is a one-line file, so the failure mode left
is forgetting it in the next job that installs something.
"""

import os
import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / ".github" / "workflows"
SCRIPT = "ci/apt-transfers.sh"


def jobs(path):
    """(job name, [step run text]) for each job in a workflow file."""
    text = path.read_text(encoding="utf-8")
    found = []
    job = None
    step_run = False
    run_lines = []
    steps = []

    def close_job():
        if job is not None:
            found.append((job, list(steps)))

    for line in text.splitlines():
        top = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if top:
            close_job()
            job = top.group(1)
            steps.clear()
            run_lines.clear()
            step_run = False
            continue
        if re.match(r"^      - ", line):
            if run_lines:
                steps.append("\n".join(run_lines))
            run_lines.clear()
            step_run = False
            continue
        if re.match(r"^        run:\s*\|", line):
            step_run = True
            continue
        if step_run:
            if line.startswith("          ") or not line.strip():
                run_lines.append(line.strip())
            else:
                step_run = False
    if run_lines:
        steps.append("\n".join(run_lines))
    close_job()
    return found


class AptTransfers(unittest.TestCase):
    def test_the_script_is_idempotent_and_sets_both_timeouts(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            conf_dir = Path(tmp) / "apt.conf.d"
            conf_dir.mkdir()
            env = dict(os.environ, APT_CONF_DIR=str(conf_dir))
            for _ in range(2):
                subprocess.run(
                    ["bash", str(ROOT / SCRIPT)], check=True, env=env, cwd=ROOT
                )
            self.assertEqual([p.name for p in conf_dir.iterdir()], ["99-ci-transfers"])
            body = (conf_dir / "99-ci-transfers").read_text()
            self.assertIn('Acquire::Retries "3"', body)
            self.assertIn('Acquire::http::Timeout "30"', body)
            self.assertIn('Acquire::https::Timeout "30"', body)

    def test_an_image_without_apt_is_left_alone(self):
        import tempfile

        env = dict(os.environ, APT_CONF_DIR="/nonexistent/apt.conf.d")
        with tempfile.TemporaryDirectory() as tmp:
            done = subprocess.run(
                ["bash", str(ROOT / SCRIPT)], env=env, cwd=tmp, capture_output=True
            )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("nothing to configure", done.stderr.decode())

    def test_every_job_that_installs_packages_arms_them_first(self):
        missing = []
        for path in sorted(WORKFLOWS.glob("*.yml")):
            for name, steps in jobs(path):
                for step in steps:
                    if "apt-get install" not in step:
                        continue
                    # Same step on purpose: a container job's apt runs inside
                    # the container, where a host-level step would not apply.
                    # Either the shared script, or the same three settings
                    # written directly by a step that runs before checkout
                    # (where the script is not in the workspace yet).
                    armed = step.find(SCRIPT)
                    if armed < 0:
                        armed = step.find('Acquire::http::Timeout')
                    if armed < 0 or step.index("apt-get install") < armed:
                        missing.append(f"{path.name}:{name}")
                    break
        self.assertEqual(
            missing,
            [],
            "these jobs install packages before arming the apt timeouts",
        )


if __name__ == "__main__":
    unittest.main()
