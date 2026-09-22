// Surface manifest for the `unstable` channel: always the whole current
// surface, no promises. Loaded (as a side module) into the framework
// snapshot; `install` runs per isolate, picked by the part's
// `//! ODM API <version>` pragma. Parts see `THREE` and `odm` as globals;
// `__odm` is engine plumbing. Stamped versions get their own manifest here
// when they are cut.
import * as THREE from '../three/entry.js';
import { installGlobals } from '../odm/index.js';

(globalThis.__odmVersions ??= {}).unstable = {
  install(g) {
    g.THREE = THREE;
    installGlobals(g);
  },
};
