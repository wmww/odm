#!/usr/bin/env sh
# Shrink target/ before CI saves it as a cache (GitHub caps a repo at 10 GB,
# shared by every lane). Keeps what is slow to rebuild — registry deps, the
# V8 prebuilt, Manifold's cmake build — and drops what is fast or useless:
# incremental/, uplifted files, and the workspace crates' own artifacts
# (seconds to rebuild; same purge as seed-target.sh).
set -eu
target=${1:-target}
locals=$(for d in crates/*/; do n=${d%/}; printf '%s ' "${n##*/}"; done)
for prof in "$target"/*/ "$target"/*/*/; do
  [ -d "$prof/deps" ] || continue
  find "$prof" -maxdepth 1 -type f -delete
  rm -rf "$prof/incremental" "$prof/examples"
  for c in $locals; do
    u=$(printf '%s' "$c" | tr - _)
    rm -rf "$prof/deps/$c"-* "$prof/deps/$u"-* "$prof/deps/lib$u"-* "$prof/deps/lib$u".* \
           "$prof/.fingerprint/$c"-* "$prof/build/$c"-*
  done
  # Integration-test binaries are named after the test file; external deps
  # never produce extensionless files.
  find "$prof/deps" -maxdepth 1 -type f ! -name '*.*' -delete
  find "$prof/deps" -maxdepth 1 -name '*.d' -delete
done
du -sh "$target"
