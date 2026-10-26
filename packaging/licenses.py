#!/usr/bin/env python3
"""Build offline, source-backed notices for both Linux Cargo graphs.

cargo-about owns SPDX expression/feature resolution; this glue retains original
notices too, and refuses its generic-text fallback when no original is available.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "packaging/licenses"
GRAPHS = {
    "melbad": ("Cargo.toml", ["LICENSE-MIT", "LICENSE-APACHE",
                             "crates/melba-cdm/third_party/cdm-api/LICENSE"]),
    "melba": ("crates/melba/Cargo.toml", ["crates/melba/LICENSE",
               "crates/melba/assets/fonts/Inter-LICENSE.txt",
               "crates/melba/assets/fonts/NotoEmoji-LICENSE.txt",
               "crates/melba/assets/icons/LICENSE.txt"]),
}
BUNDLES = tuple(f"{name}-THIRD-PARTY.txt" for name in GRAPHS)
RECEIPT = "receipt.json"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def walk(directory):
    for base, dirs, files in os.walk(directory):
        dirs[:] = sorted(d for d in dirs if d not in ("target", ".git"))
        for name in sorted(files):
            yield Path(base) / name


def notice_name(path):
    name = path.name.upper()
    return (name.startswith(("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE"))
            and path.suffix.lower() in ("", ".txt", ".md", ".html")) or name in ("OFL.TXT", "UFL.TXT")


def original_files(package):
    directory = Path(package["manifest_path"]).parent
    # A developer checkout may also contain makepkg's src/pkg trees. The
    # project's vendored assets are explicit below; don't scan build products.
    candidates = directory.iterdir() if directory == ROOT else walk(directory)
    files = {p for p in candidates if p.is_file() and notice_name(p)}
    # Local subcrates share this source root's notices even in a source archive.
    # For upstream Git workspaces, stop at their checkout boundary; never borrow
    # a neighbouring registry package's license.
    if package["source"] is None or package["source"].startswith("git+"):
        parents = (directory, *directory.parents)
        if package["source"] is None:
            # Local workspace subcrates share this checkout's notices. A
            # local path dependency outside the checkout owns only its own
            # directory; an unrelated .git ancestor (for example a temporary
            # fixture) must not lend the project root's LICENSE to it.
            boundary = ROOT if directory.is_relative_to(ROOT) else directory
        else:
            boundary = next((p for p in parents if (p / ".git").exists()), directory)
        for parent in parents:
            files.update(p for p in parent.iterdir() if p.is_file() and notice_name(p))
            if parent == boundary:
                break
    return sorted(files)


def validate_report(report):
    packages = {entry["package"]["id"]: entry["package"] for entry in report["crates"]}
    if not packages or not report["licenses"]:
        raise ValueError("empty dependency/license report")
    covered = set()
    for license in report["licenses"]:
        if not license["text"].strip() or not license["used_by"]:
            raise ValueError("empty license text or attribution")
        covered.update(use["crate"]["id"] for use in license["used_by"])
    if covered != packages.keys():
        raise ValueError(f"incomplete license coverage: {sorted(packages.keys() ^ covered)}")
    return packages


def render(report, supplements, data_dir=DATA):
    packages = validate_report(report)
    upstream = {entry["package"]: entry for entry in supplements["upstream"]}
    extras = {}
    for entry in supplements["extra_files"]:
        extras.setdefault(entry["package"], []).append(entry)
    originals = {}
    source_files = {}
    for license in report["licenses"]:
        if license.get("source_path"):
            source_path = Path(license["source_path"])
            for use in license["used_by"]:
                # cargo-about may group several packages under one license
                # entry. A source notice belongs only to the package whose
                # checkout contains it; the root project's LICENSE must not
                # become an upstream dependency's original notice.
                package_dir = Path(packages[use["crate"]["id"]]["manifest_path"]).parent
                if source_path.is_relative_to(package_dir):
                    source_files.setdefault(use["crate"]["id"], set()).add(source_path)
    # Keep the whole source notice, even when a scanner selected only one
    # clause of a combined license (notably AWS-LC, ring and font notices).
    for identifier, package in sorted(packages.items()):
        files = sorted(set(original_files(package)) | source_files.get(identifier, set()))
        texts = [(str(p.relative_to(Path(package["manifest_path"]).parent))
                  if p.is_relative_to(Path(package["manifest_path"]).parent)
                  else p.name, p.read_bytes()) for p in files]
        if identifier in upstream:
            entry = upstream[identifier]
            content = (data_dir / entry["path"]).read_bytes()
            if digest(content) != entry["sha256"]:
                raise ValueError(f"supplement checksum mismatch: {identifier}")
            texts.append((entry["url"], content))
        for entry in extras.get(identifier, []):
            path = Path(package["manifest_path"]).parent / entry["path"]
            content = path.read_bytes()
            if digest(content) != entry["sha256"]:
                raise ValueError(f"embedded asset notice changed: {identifier}")
            texts.append((entry["path"], content))
        if not texts or any(not content.strip() for _, content in texts):
            raise ValueError(f"no original notice for {identifier}; curate a pinned supplement")
        originals[identifier] = texts

    def label(identifier):
        package = packages[identifier]
        if package["source"] is None:
            return f"{package['name']}@{package['version']} (local project)"
        return identifier

    chunks = ["Third-party notices — Linux default-feature build (including build dependencies)",
              "SPDX choices: packaging/licenses/about.toml. Original notices follow verbatim.",
              "Unmodified MPL-2.0 component source is available at the exact-version download URLs below.",
              "", "DEPENDENCY INDEX"]
    for identifier, package in sorted(packages.items()):
        source = package["source"]
        if source and source.startswith("registry+"):
            source = f"https://crates.io/api/v1/crates/{package['name']}/{package['version']}/download"
        chunks.append(f"{label(identifier)}\nSource: {source or 'this source checkout'}")
    chunks.append("\nSELECTED LICENSE TERMS")
    for license in report["licenses"]:
        users = sorted(label(use["crate"]["id"]) for use in license["used_by"])
        chunks.extend([f"\n=== {license['id']} — {', '.join(users)} ===", license["text"]])
    chunks.append("\nORIGINAL COPYRIGHT / LICENSE / NOTICE FILES")
    # Deduplicate identical text without losing the list of packages it covers.
    grouped = {}
    for identifier, texts in originals.items():
        for origin, content in texts:
            key = digest(content)
            group = grouped.setdefault(key, {"content": content, "origins": set()})
            group["origins"].add(f"{label(identifier)}: {origin}")
    for key, group in sorted(grouped.items()):
        chunks.extend([f"\n=== SHA256 {key} ===", *sorted(group["origins"]),
                       group["content"].decode("utf-8")])
    return "\n".join(chunks) + "\n"


def inputs():
    paths = {ROOT / "Cargo.toml", ROOT / "Cargo.lock", Path(__file__).resolve()}
    paths.update(p for p in ROOT.iterdir() if p.is_file() and notice_name(p))
    paths.update(p for p in walk(ROOT / "crates")
                 if p.name in ("Cargo.toml", "Cargo.lock") or notice_name(p))
    paths.update(p for p in walk(DATA))
    for _, assets in GRAPHS.values():
        paths.update(ROOT / asset for asset in assets)
    return {str(p.relative_to(ROOT)): digest(p.read_bytes()) for p in sorted(paths)}


def check(output):
    receipt = json.loads((output / RECEIPT).read_text())
    if receipt["schema"] != 1 or receipt["inputs"] != inputs():
        raise ValueError("stale notices: source manifests, locks or license inputs changed; regenerate")
    if set(receipt["bundles"]) != set(BUNDLES):
        raise ValueError("incomplete notice bundle set")
    for filename, sha in receipt["bundles"].items():
        if digest((output / filename).read_bytes()) != sha:
            raise ValueError(f"notice bundle checksum mismatch: {filename}")


def generate(output):
    before = inputs()
    supplements = json.loads((DATA / "supplements.json").read_text())
    tool = subprocess.run(["cargo-about", "--version"], check=True, capture_output=True,
                          text=True).stdout.strip()
    contents = {}
    for name, (manifest, assets) in GRAPHS.items():
        command = ["cargo-about", "--color", "never", "generate", "--frozen", "--fail",
                   "--config", str(DATA / "about.toml"), "--format", "json",
                   "--manifest-path", str(ROOT / manifest)]
        result = subprocess.run(command, cwd=ROOT, check=True, capture_output=True, text=True)
        report = json.loads(result.stdout)
        text = render(report, supplements)
        text += "\nVENDORED PROJECT / ASSET NOTICES\n"
        for asset in assets:
            text += f"\n=== {asset} ===\n" + (ROOT / asset).read_text()
        contents[f"{name}-THIRD-PARTY.txt"] = text.encode("utf-8")
        print(f"{name}: {len(report['crates'])} packages, source-backed notices collected")
    if inputs() != before:
        raise ValueError("license inputs changed during generation")
    receipt = {"schema": 1, "tool": tool, "inputs": before,
               "bundles": {name: digest(data) for name, data in contents.items()}}
    output.mkdir(parents=True, exist_ok=True)
    # Publish the receipt last. An interrupted/failed generation cannot bless
    # stale or half-written bundles, and package()/install always re-check it.
    contents[RECEIPT] = (json.dumps(receipt, sort_keys=True, indent=2) + "\n").encode()
    for name, data in contents.items():
        with tempfile.NamedTemporaryFile(dir=output, delete=False) as temporary:
            temporary.write(data)
        os.replace(temporary.name, output / name)
    check(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--check", action="store_true", help="check freshness/integrity without cargo-about")
    args = parser.parse_args()
    try:
        if args.check:
            check(args.output)
        else:
            generate(args.output)
    except subprocess.CalledProcessError as error:
        print(error.stderr or str(error), file=sys.stderr)
        return 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"licenses: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
