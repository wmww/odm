# Hermetic builds: manifold-csg-sys clones at build time

`manifold-csg-sys` (0.3.3) runs `git clone` of pinned upstream Manifold (v3.5.1)
during `cargo build` → fresh builds need network; breaks offline/sandboxed CI and
Nix-style builds. Escape hatch: `MANIFOLD_CSG_LIB_DIR` env var pointing at a prebuilt
lib.

Accepted for MVP (CI runners have network). Revisit when setting up CI caching or if
offline builds matter: options are vendoring the upstream source (submodule or copy),
prebuilding + `MANIFOLD_CSG_LIB_DIR`, or upstreaming a vendored-source feature to the
binding crate.
