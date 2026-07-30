// Surface manifest for the test-only `test` API version: unstable's
// globals plus `odm.apiProbe` (see ./odm.js).
import * as THREE from '../../three/entry.js';
import { installGlobals } from '../../odm/index.js';
import { apiProbe } from './odm.js';

(globalThis.__odmVersions ??= {}).test = {
  install(g) {
    g.THREE = THREE;
    installGlobals(g);
    g.odm.apiProbe = apiProbe;
  },
};
