// Bare-'odm' surface for the test-only `test` API version: unstable plus
// one deliberate difference (`apiProbe`), so tests can prove that pragma
// routing, version coexistence, and cross-version invoke actually work.
// Only reachable in builds with the odm-js `test-api-version` feature.
export * from '../../odm/index.js';

/** The deliberate surface difference: absent on every real version. */
export function apiProbe() {
  return 'test';
}
