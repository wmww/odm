# Geometry built at module top level works natively, throws on web export

`framework/odm/index.js`'s `ops()` throws "ODM engine ops unavailable:
this code only runs inside a build" when neither `globalThis.__odmOps`
nor `Deno.core.ops` is there. On the desktop engine `Deno.core.ops` is
present for the whole isolate, so module-scope `odm.box(1)` builds fine
(verified 2026-09-09, both through `build()` and through the meta
extraction path). The web export installs `__odmOps` per build
(`crates/odm-web/src/executor.rs`), so the same file throws in a browser.

So a doohickey can pass every local check and break only once exported —
the failure mode `notes/web-export.md` calls out as the drift risk, here
in the framework rather than in the ops list.

Options: install the ops for the isolate's lifetime in odm-web too (they
are per-build only because the session context is), or make the native
side fail the same way so the divergence shows up locally. The second is
the honest one — module scope should not build geometry, because a
memoized rebuild re-evaluates the module and a *cached* one does not.

docs/api/errors.md now describes the split; it previously claimed the
error fires natively, which it does not.
