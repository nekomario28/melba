#!/usr/bin/env python3
"""Fail closed when an ELF executable requires a newer GLIBC symbol than the release ABI."""
import re
import subprocess
import sys

SYMBOL = re.compile(r'\bGLIBC_(\d+)\.(\d+)(?:\.(\d+))?\b')


def required_glibc(readelf_output):
    values = [
        (int(m.group(1)), int(m.group(2)), int(m.group(3) or 0))
        for m in SYMBOL.finditer(readelf_output)
    ]
    if not values:
        raise ValueError("binary has no GLIBC version requirements; cannot establish GNU ABI baseline")
    return max(values)


def main(args):
    if len(args) < 2:
        print("usage: check_glibc_abi.py MAX_GLIBC (ELF_BIN...)", file=sys.stderr)
        return 2
    try:
        limit = tuple(int(x) for x in args[0].split("."))
        if len(limit) == 2:
            limit += (0,)
        if len(limit) != 3:
            raise ValueError("baseline version must be major.minor[.patch]")
        for binary in args[1:]:
            data = subprocess.run(
                ["readelf", "-W", "--version-info", binary],
                text=True, capture_output=True, check=True
            ).stdout
            actual = required_glibc(data)
            dotted = ".".join(str(x) for x in actual[:2])
            print(f"{binary}: maximum required GLIBC_{dotted}; allowed <= GLIBC_{args[0]}")
            if actual > limit:
                print(f"ERROR: {binary} exceeds release glibc baseline", file=sys.stderr)
                return 1
    except (OSError, subprocess.CalledProcessError, ValueError) as exc:
        print(f"ERROR: cannot validate glibc ABI: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
