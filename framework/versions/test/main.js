// Surface manifest for the test-only `test` API version: unstable's
// globals plus `odm.apiProbe`, the one deliberate surface difference, so
// tests can prove that pragma routing and version coexistence actually
// work. Only reachable in builds with the odm-js `test-api-version` feature.
import * as THREE from '../../three/entry.js';
import { installGlobals } from '../../odm/index.js';

(globalThis.__odmVersions ??= {}).test = {
  install(g) {
    g.THREE = THREE;
    installGlobals(g);
    g.odm.apiProbe = () => 'test';
  },
};
