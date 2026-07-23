// Snapshot entry (loaded as a SIDE module at snapshot build time).
// Doohickeys see `THREE` and `odm` as globals; `__odm` is engine plumbing.
import * as THREE from './three/entry.js';
import { installGlobals } from './odm/index.js';

globalThis.THREE = THREE;
installGlobals(globalThis);
