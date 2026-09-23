#!/usr/bin/env sh
# Materialize the patched crates that Cargo.toml's [patch.crates-io] points at:
# download the exact crates.io release, check its checksum, apply
# patches/<name>-<version>.patch, into vendor/<name> (gitignored). Run once
# after cloning and again whenever a patch changes; cargo fails with a missing
# vendor/<name>/Cargo.toml until then. Idempotent: a stamp records the patch.
#
# deno_core: the snapshot use-after-free that aborts macOS runs (see
# notes/architecture.md, Platforms ▸ macOS). Drop the entry, the [patch] line
# and the patch file once an upstream release fixes it.
set -eu

top=$(cd "$(dirname "$0")/.." && pwd)

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# name version sha256 (the crates.io checksum, as Cargo.lock had it)
materialize() {
  name=$1 version=$2 want=$3
  patch="$top/patches/$name-$version.patch"
  dest="$top/vendor/$name"
  stamp="$(sha256 "$patch") $version"
  if [ -f "$dest/.odm-patch-stamp" ] && [ "$(cat "$dest/.odm-patch-stamp")" = "$stamp" ]; then
    return
  fi
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  curl -fsSL "https://static.crates.io/crates/$name/$name-$version.crate" -o "$tmp/crate.tgz"
  got=$(sha256 "$tmp/crate.tgz")
  if [ "$got" != "$want" ]; then
    echo "patch-deps: $name $version checksum mismatch: $got" >&2
    exit 1
  fi
  tar -xzf "$tmp/crate.tgz" -C "$tmp"
  # Outside any repo, git apply is a plain (and strict) patch tool.
  (cd "$tmp/$name-$version" && git apply -p1 "$patch")
  rm -rf "$dest"
  mkdir -p "$top/vendor"
  mv "$tmp/$name-$version" "$dest"
  echo "$stamp" > "$dest/.odm-patch-stamp"
  echo "patch-deps: vendor/$name ($version + patch)"
}

materialize deno_core 0.408.0 9c1d2bf3bf9a7dd3f4184cf3af56ce9f06b7bb9cab93497ec3acd04196c9db3f
