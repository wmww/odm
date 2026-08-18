#!/usr/bin/env sh
# Build odm and install it for the current user: the binary (framework and
# docs are embedded) plus the web-export template, which `odm export --web`
# looks up at ~/.local/share/odm/web-template.bin.
#
#   scripts/install.sh              -> ~/.local/bin/odm
#   BINDIR=~/bin scripts/install.sh -> ~/bin/odm
#   DATADIR overrides ~/.local/share/odm (the template's home; keep it
#   where the binary looks unless you point ODM_WEB_TEMPLATE at it)
#
# Builds in the repo's target/ dir, so it reuses whatever is already compiled
# (unlike `cargo install --path crates/odm`, which builds from scratch).
# The template build needs the wasm toolchain (clang + wasm-ld + libc++
# headers, wasm-bindgen-cli) — see notes/web-export.md. xtask picks up a
# rootless one from ~/.local/opt/wasm-cxx and says what is missing otherwise.
set -eu

top=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
bindir=${BINDIR:-$HOME/.local/bin}
datadir=${DATADIR:-$HOME/.local/share/odm}

# --no-default-features: the default `dynamic` feature links against
# libodm_dylib.so in the target dir; an installed binary must be self-contained.
cargo build --release --manifest-path "$top/Cargo.toml" --bin odm --no-default-features
cargo run -q --release --manifest-path "$top/Cargo.toml" -p xtask -- build-web-template

mkdir -p "$bindir"
install -m755 "$top/target/release/odm" "$bindir/odm"
echo "installed $bindir/odm"

# One template file, one static name: installing replaces the previous one
# (the stamp inside it is what `odm export` checks). The rm clears the
# stamped-directory layout an earlier install.sh used.
mkdir -p "$datadir"
rm -rf "$datadir/web-template"
install -m644 "$top/target/web-template.bin" "$datadir/web-template.bin"
echo "installed web-export template at $datadir/web-template.bin"

case ":$PATH:" in
  *":$bindir:"*) ;;
  *) echo "warning: $bindir is not on your PATH" >&2 ;;
esac
