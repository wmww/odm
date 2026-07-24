#!/usr/bin/env python3
"""Prune orphaned artifacts from a cargo target dir.

cargo never garbage-collects: every profile edit, dep bump, rustc upgrade, or
build-script rerun leaves the previous artifacts behind forever. cargo-sweep
prunes by mtime, which is useless on a young target dir (everything is "recent"
until suddenly everything is "old"). This instead asks cargo which artifacts the
current builds actually reference, and deletes only what nothing points at.

Usage:
  scripts/sweep-target.py --dry-run            # report, delete nothing
  scripts/sweep-target.py                      # prune
  scripts/sweep-target.py --keep-incremental   # don't drop incremental/
Sweeps this checkout by default; pass checkout paths to sweep others too.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

HASH = re.compile(r"-([0-9a-f]{8,32})(?:\.|$)")


def hash_of(name: str):
    """Trailing -<hex> disambiguator cargo appends to artifact and unit names."""
    m = HASH.search(name)
    return m.group(1) if m else None


def target_dir(checkout: Path) -> Path:
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=checkout, capture_output=True, text=True, check=True,
    )
    return Path(json.loads(out.stdout)["target_directory"])


def live_hashes(checkout: Path):
    """Hashes of every unit the current build graph references, per cargo itself."""
    proc = subprocess.run(
        ["cargo", "build", "--workspace", "--all-targets", "--message-format=json"],
        cwd=checkout, capture_output=True, text=True,
    )
    if proc.returncode != 0:
        sys.exit(f"cargo build failed in {checkout} — refusing to sweep:\n"
                 f"{proc.stderr[-2000:]}")
    hashes, files = set(), set()
    for line in proc.stdout.splitlines():
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        reason = msg.get("reason")
        if reason == "compiler-artifact":
            for f in msg.get("filenames") or []:
                files.add(Path(f))
                if h := hash_of(Path(f).name):
                    hashes.add(h)
            # build-script binaries live in build/<pkg>-<hash>/
            for f in msg.get("filenames") or []:
                for part in Path(f).parts:
                    if h := hash_of(part):
                        hashes.add(h)
        elif reason == "build-script-executed":
            if out := msg.get("out_dir"):
                # .../build/<pkg>-<hash>/out
                if h := hash_of(Path(out).parent.name):
                    hashes.add(h)
    return hashes, files


def sweep(root: Path, live: set, dry: bool, drop_incremental: bool):
    freed, kept, removed = 0, 0, []

    def size(p: Path):
        if p.is_file():
            return p.stat().st_size
        return sum(f.stat().st_size for f in p.rglob("*") if f.is_file())

    for profile_dir in root.iterdir():
        if not profile_dir.is_dir() or profile_dir.name.startswith("."):
            continue
        # hash-keyed unit dirs and artifact files
        for sub in ("deps", "build", ".fingerprint"):
            d = profile_dir / sub
            if not d.is_dir():
                continue
            for entry in d.iterdir():
                h = hash_of(entry.name)
                if h is None or h in live:
                    kept += 1
                    continue
                freed += size(entry)
                removed.append(entry)
                if not dry:
                    shutil.rmtree(entry) if entry.is_dir() else entry.unlink()
        inc = profile_dir / "incremental"
        if drop_incremental and inc.is_dir():
            freed += size(inc)
            removed.append(inc)
            if not dry:
                shutil.rmtree(inc)
    return freed, kept, removed


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("checkouts", nargs="*", type=Path)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--keep-incremental", action="store_true")
    args = ap.parse_args()

    checkouts = args.checkouts or [Path(__file__).resolve().parent.parent]

    busy = subprocess.run(["pgrep", "-x", "rustc"], capture_output=True).returncode == 0
    if busy and not args.dry_run:
        sys.exit("rustc is running — another build is in flight; try again later.")

    # Checkouts keep separate target dirs, so each root is swept against its own
    # live set. (Two checkouts still land on one root if CARGO_TARGET_DIR says so.)
    roots = {}
    seen = set()
    for c in checkouts:
        c = c.resolve()
        if c in seen or not (c / "Cargo.toml").exists():
            continue
        seen.add(c)
        root = target_dir(c)
        h, _ = live_hashes(c)
        print(f"{c}: {len(h)} live units -> {root}")
        roots.setdefault(root, set()).update(h)

    total = 0
    for root, live in roots.items():
        before = sum(f.stat().st_size for f in root.rglob("*") if f.is_file())
        freed, kept, removed = sweep(root, live, args.dry_run,
                                     not args.keep_incremental)
        total += freed
        verb = "would free" if args.dry_run else "freed"
        print(f"{root}: {before/2**30:.1f} GiB -> {verb} {freed/2**30:.1f} GiB "
              f"({len(removed)} orphans removed, {kept} kept)")
    print(f"total {'would free' if args.dry_run else 'freed'}: {total/2**30:.1f} GiB")


if __name__ == "__main__":
    main()
