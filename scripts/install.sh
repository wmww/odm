#!/usr/bin/env sh
# Build odm and install it for the current user. The binary is self-contained
# (framework, docs and the web-export template are embedded).
#
#   scripts/install.sh              -> ~/.local/bin/odm
#   BINDIR=~/bin scripts/install.sh -> ~/bin/odm
#
# Builds in the repo's target/ dir, so it reuses whatever is already compiled
# (unlike `cargo install --path crates/odm`, which builds from scratch).
# The template build needs the wasm toolchain (clang + wasm-ld + libc++
# headers, wasm-bindgen-cli) — see notes/web-export.md. xtask picks up a
# rootless one from ~/.local/opt/wasm-cxx and says what is missing otherwise.
set -eu

top=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
bindir=${BINDIR:-$HOME/.local/bin}

# Template first: the odm build embeds target/web-template.bin.
cargo run -q --release --manifest-path "$top/Cargo.toml" -p xtask -- build-web-template
# --no-default-features: the default `dynamic` feature links against
# libodm_dylib.so in the target dir; an installed binary must be self-contained.
cargo build --release --manifest-path "$top/Cargo.toml" --bin odm --no-default-features

mkdir -p "$bindir"
install -m755 "$top/target/release/odm" "$bindir/odm"
echo "installed $bindir/odm"
# Earlier installs kept the template as a separate file; nothing reads it now.
rm -rf "$HOME/.local/share/odm/web-template.bin" "$HOME/.local/share/odm/web-template"

case ":$PATH:" in
  *":$bindir:"*) ;;
  *) echo "warning: $bindir is not on your PATH" >&2 ;;
esac
