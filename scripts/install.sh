#!/usr/bin/env sh
# Build odm and install it for the current user. The binary is self-contained
# (framework and docs are embedded), so this is the whole install.
#
#   scripts/install.sh              -> ~/.local/bin/odm
#   BINDIR=~/bin scripts/install.sh -> ~/bin/odm
#
# Builds in the repo's target/ dir, so it reuses whatever is already compiled
# (unlike `cargo install --path crates/odm`, which builds from scratch).
set -eu

top=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
bindir=${BINDIR:-$HOME/.local/bin}

cargo build --release --manifest-path "$top/Cargo.toml" --bin odm
mkdir -p "$bindir"
install -m755 "$top/target/release/odm" "$bindir/odm"
echo "installed $bindir/odm"

case ":$PATH:" in
  *":$bindir:"*) ;;
  *) echo "warning: $bindir is not on your PATH" >&2 ;;
esac
