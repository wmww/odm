#!/usr/bin/env sh
# Build odm and install it for the current user: the binary (framework and
# docs are embedded) plus the web-export template, which `odm export --web`
# looks up in ~/.local/share/odm/web-template/<stamp>/.
#
#   scripts/install.sh              -> ~/.local/bin/odm
#   BINDIR=~/bin scripts/install.sh -> ~/bin/odm
#   DATADIR overrides ~/.local/share/odm (the template's home; keep it
#   where the binary looks unless you point ODM_WEB_TEMPLATE at it)
#
# Builds in the repo's target/ dir, so it reuses whatever is already compiled
# (unlike `cargo install --path crates/odm`, which builds from scratch).
# The template build needs the wasm toolchain (clang + wasm-ld + libc++
# headers, wasm-bindgen-cli) — see notes/web-export.md; xtask says what is
# missing if something is.
set -eu

top=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
bindir=${BINDIR:-$HOME/.local/bin}
datadir=${DATADIR:-$HOME/.local/share/odm}

cargo build --release --manifest-path "$top/Cargo.toml" --bin odm
cargo run -q --release --manifest-path "$top/Cargo.toml" -p xtask -- build-web-template

mkdir -p "$bindir"
install -m755 "$top/target/release/odm" "$bindir/odm"
echo "installed $bindir/odm"

# The template is stamp-addressed: the exported dir name must match the
# stamp baked into the binary just built (same tree, same run).
stamp=$(sed -n 's/.*"stamp": "\([0-9a-f]*\)".*/\1/p' "$top/target/web-template/template.json")
[ -n "$stamp" ] || { echo "error: no stamp in target/web-template/template.json" >&2; exit 1; }
tpldir=$datadir/web-template/$stamp
rm -rf "$tpldir"
mkdir -p "$tpldir"
cp "$top"/target/web-template/* "$tpldir/"
echo "installed web-export template at $tpldir"

case ":$PATH:" in
  *":$bindir:"*) ;;
  *) echo "warning: $bindir is not on your PATH" >&2 ;;
esac
