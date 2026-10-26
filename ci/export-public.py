#!/usr/bin/env python3
"""Build a history-free source export for the public `melba` repository.

Reads HEAD only (never the private history), removes nonempty extracted-string
outputs, records their identities as public exclusions, rewrites repository URLs
and leaves the result uncommitted for review. No network, no payload.
"""
import json
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OLD = "nekomario28/melba"
NEW = "nekomario28/melba"
EXTRACT_DIR = Path("evidence/native-scan")
KEEP_PROVENANCE_URL = Path("materials/provenance.json")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: ci/export-public.py DEST", file=sys.stderr)
        return 2
    dest = Path(sys.argv[1]).resolve()
    if dest.exists() and any(dest.iterdir()):
        print(f"refusing non-empty destination: {dest}", file=sys.stderr)
        return 2
    dirty = subprocess.run(["git", "-C", str(ROOT), "status", "--porcelain"],
                           capture_output=True, text=True, check=True).stdout
    if dirty.strip():
        print("refusing a dirty worktree; commit first", file=sys.stderr)
        return 2
    dest.mkdir(parents=True, exist_ok=True)
    archive = subprocess.run(["git", "-C", str(ROOT), "archive", "--format=tar", "HEAD"],
                             capture_output=True, check=True).stdout
    with tarfile.open(fileobj=__import__("io").BytesIO(archive)) as tar:
        tar.extractall(dest)
    # Remove nonempty extract outputs, keep empty ones as evidence of no match.
    removed = []
    for path in sorted((dest / EXTRACT_DIR).glob("*.txt")):
        if path.stat().st_size:
            data = path.read_bytes()
            removed.append({
                "path": str(path.relative_to(dest)),
                "sha256": __import__("hashlib").sha256(data).hexdigest(),
                "bytes": len(data),
                "category": "extracted_strings",
                "origin_status": "reported_derivation_not_reproduced",
                "source": "android_scan_report",
                "reason": "nonempty extracted library strings withheld from the public export; retained in private evidence",
            })
            path.unlink()
    ledger_path = dest / "materials/provenance.json"
    ledger = json.loads(ledger_path.read_text())
    ledger["records"] = [r for r in ledger["records"]
                         if not (r.get("category") == "extracted_strings" and r.get("origin_status") != "empty_extraction_output")]
    ledger["public_exclusions"] = removed
    ledger_path.write_text(json.dumps(ledger, indent=2, ensure_ascii=False) + "\n")
    # Point release/docs metadata at the clean public repository, but keep the
    # historical provenance repository factual.
    changed = 0
    for path in dest.rglob("*"):
        if not path.is_file() or path == ledger_path or ".git" in path.parts:
            continue
        try:
            text = path.read_text()
        except (UnicodeDecodeError, OSError):
            continue
        if OLD in text:
            path.write_text(text.replace(OLD, NEW))
            changed += 1
    readme = dest / "materials/README.md"
    text = readme.read_text()
    old = """Extracted-string filenames do not make their contents working endpoints.
The retained outputs include domain suffixes, certificate URLs with extraction
noise, JNI/C++ symbols, diagnostics and SQL fragments. None was sent as a
request in this review. An empty extraction output says nothing about the
absence of an API in the source library. Historical reports identify an APK
and a method, but this pass did not reacquire that APK or reproduce extraction.
Those reports' old capability and legal conclusions are not current verdicts."""
    new = """Extracted-string filenames do not make their contents working endpoints.
The **nonempty** outputs (domain suffixes, certificate text, JNI/C++ symbols,
diagnostics and SQL fragments) are withheld from this public export and retained
privately; their identities are listed in `public_exclusions`. The ten empty
outputs remain, but an empty extraction says nothing about the absence of an API
in the source library. None was sent as a request. Historical reports identify an
APK and a method, but this pass did not reacquire that APK or reproduce
extraction. Those reports' old capability and legal conclusions are not current
verdicts."""
    if old not in text:
        print("material README paragraph changed; update exporter", file=sys.stderr)
        return 1
    readme.write_text(text.replace(old, new))
    print(f"PUBLIC_EXPORT_WRITTEN {dest} files={sum(1 for p in dest.rglob('*') if p.is_file())} url_files={changed} withheld={len(removed)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
