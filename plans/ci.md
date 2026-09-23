# Plan: CI — remaining GitHub-side steps

Written 2026-09-22; the local half is done (workflows, image, timeout
scaling, adapter assertion, real_adapters test, docs — see architecture.md
"CI" and build-environment.md "CI lanes"). What is left needs the commit on
GitHub's default branch (`workflow_dispatch` only sees workflows there):

1. `gh workflow run container.yml`; then make the `odm-build` GHCR package
   public (package settings → visibility; no API for it).
2. `gh workflow run run.yml -f lane=<each of the four> -f command='cargo
   --version; rustc -vV; df -h .; nproc'` — the self-test, and the first
   look at the Windows/macOS runners: record what it shows under each
   lane's heading in build-environment.md.
3. `gh workflow run test.yml` (both Linux lanes), then once more to confirm
   the cache hits. Record cold/warm timings and the saved cache size. arm64
   has never run: expect float tolerances in precision/render asserts to
   need loosening (loosen, don't fork goldens). If arm64 is much slower,
   default `lanes` to x86_64.
4. `test.yml -f real_adapters=true` once.
5. Delete this plan.
