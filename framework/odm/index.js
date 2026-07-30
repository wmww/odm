// ODM framework: the API doohickeys build with.
//
// Conventions: Z-up, distances in project units, ALL angles in radians
// (like three.js; `odm.deg(90)` converts). Solids are immutable — every
// method returns a new value. Geometry lives engine-side; a Solid holds an
// opaque content-hash handle plus a pending transform and color.
import * as THREE from '../three/entry.js';
import { parseColor } from './colors.js';

function ops() {
  const o = globalThis.Deno?.core?.ops;
  if (!o || !o.op_solid_box) {
    throw new Error('ODM engine ops unavailable: this code only runs inside a build');
  }
  return o;
}

// ---------- matrices ----------

const IDENTITY = Object.freeze([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);

function matElements(m) {
  if (m === null) return IDENTITY;
  return m.elements;
}

function isIdentity(m) {
  return m === null || m.elements.every((v, i) => v === IDENTITY[i]);
}

function toMatrix4(m) {
  if (m instanceof THREE.Matrix4) return m.clone();
  if (Array.isArray(m) && m.length === 16) return new THREE.Matrix4().fromArray(m);
  throw new TypeError('transform() takes a THREE.Matrix4 or a column-major array of 16 numbers');
}

/** Left-multiply: apply `extra` (in world frame) after the existing matrix. */
function premul(matrix, extra) {
  const base = matrix === null ? new THREE.Matrix4() : matrix;
  return extra.clone().multiply(base);
}

// ---------- scene values: Solid, Group, Instance ----------

const transformable = (Base) =>
  class extends Base {
    translate(x = 0, y = 0, z = 0) {
      return this._with({ matrix: premul(this._matrix, new THREE.Matrix4().makeTranslation(x, y, z)) });
    }
    rotateX(rad) {
      return this._with({ matrix: premul(this._matrix, new THREE.Matrix4().makeRotationX(rad)) });
    }
    rotateY(rad) {
      return this._with({ matrix: premul(this._matrix, new THREE.Matrix4().makeRotationY(rad)) });
    }
    rotateZ(rad) {
      return this._with({ matrix: premul(this._matrix, new THREE.Matrix4().makeRotationZ(rad)) });
    }
    /** Rotate around an arbitrary axis ([x,y,z] or Vector3) through the origin. */
    rotate(axis, rad) {
      const a = Array.isArray(axis) ? new THREE.Vector3(...axis) : axis.clone();
      return this._with({
        matrix: premul(this._matrix, new THREE.Matrix4().makeRotationAxis(a.normalize(), rad)),
      });
    }
    scale(x, y, z) {
      if (y === undefined) [y, z] = [x, x];
      return this._with({ matrix: premul(this._matrix, new THREE.Matrix4().makeScale(x, y, z)) });
    }
    transform(m) {
      return this._with({ matrix: premul(this._matrix, toMatrix4(m)) });
    }
    color(c) {
      return this._with({ color: parseColor(c) });
    }
    name(n) {
      return this._with({ name: String(n) });
    }
  };

class SceneValue {
  constructor(matrix, color, label) {
    this._matrix = matrix; // THREE.Matrix4 | null (identity)
    this._color = color; // linear [r,g,b,a] | null
    this._name = label; // string | null
  }
}

/** A solid body: an engine-side geometry handle + pending transform/color. */
export class Solid extends transformable(SceneValue) {
  constructor(geom, matrix = null, color = null, label = null) {
    super(matrix, color, label);
    this._geom = geom; // content-hash hex string
    this._bakedCache = null;
  }

  _with({ matrix = this._matrix, color = this._color, name = this._name }) {
    return new Solid(this._geom, matrix, color, name);
  }

  get geometry() {
    return this._geom;
  }

  _operand() {
    return { geom: this._geom, matrix: matElements(this._matrix) };
  }

  static _bool(kind, solids, keep) {
    for (const s of solids) {
      if (!(s instanceof Solid)) {
        throw new TypeError(
          `${kind} operands must be Solids (got ${s?.constructor?.name ?? typeof s}); ` +
            'Groups/Instances cannot be used in CSG',
        );
      }
    }
    const geom = ops().op_boolean(kind, solids.map((s) => s._operand()));
    return new Solid(geom, null, keep._color, keep._name);
  }

  union(...others) {
    return Solid._bool('union', [this, ...others.flat()], this);
  }
  add(...others) {
    return this.union(...others);
  }
  subtract(...others) {
    return Solid._bool('difference', [this, ...others.flat()], this);
  }
  intersect(...others) {
    return Solid._bool('intersection', [this, ...others.flat()], this);
  }
  hull(...others) {
    const all = [this, ...others.flat()];
    const geom = ops().op_hull(all.map((s) => s._operand()));
    return new Solid(geom, null, this._color, this._name);
  }

  /** Bake the pending transform into the geometry (usually not needed). */
  bake() {
    return new Solid(this._baked(), null, this._color, this._name);
  }

  _baked() {
    if (isIdentity(this._matrix)) return this._geom;
    if (this._bakedCache === null) {
      this._bakedCache = ops().op_transform_bake(this._geom, matElements(this._matrix));
    }
    return this._bakedCache;
  }

  volume() {
    return ops().op_volume(this._baked());
  }
  area() {
    return ops().op_area(this._baked());
  }
  /** Axis-aligned bounds in this solid's (world) frame: {min, max} or null if empty. */
  bounds() {
    return ops().op_bounds(this._baked());
  }
  /**
   * Nearest surface hit of the ray from `origin` along `dir`, or null.
   * Returns { distance, position: [x,y,z], normal: [x,y,z] }.
   */
  raycast(origin, dir, maxDist = 1e9) {
    const o = Array.isArray(origin) ? origin : [origin.x, origin.y, origin.z];
    const d = Array.isArray(dir) ? dir : [dir.x, dir.y, dir.z];
    return ops().op_raycast(this._baked(), o, d, maxDist);
  }

  _toIR() {
    return {
      ...(this._name !== null && { name: this._name }),
      ...(!isIdentity(this._matrix) && { matrix: [...matElements(this._matrix)] }),
      ...(this._color !== null && { color: this._color }),
      geom: this._geom,
    };
  }
}

/** A pure grouping of scene values under one transform/color. */
export class Group extends transformable(SceneValue) {
  constructor(children, matrix = null, color = null, label = null) {
    super(matrix, color, label);
    this._children = children;
  }

  _with({ matrix = this._matrix, color = this._color, name = this._name }) {
    return new Group(this._children, matrix, color, name);
  }

  get children() {
    return [...this._children];
  }

  _toIR() {
    return {
      ...(this._name !== null && { name: this._name }),
      ...(!isIdentity(this._matrix) && { matrix: [...matElements(this._matrix)] }),
      ...(this._color !== null && { color: this._color }),
      children: this._children.map(toIRNode),
    };
  }
}

/** The output of ctx.invoke(): another doohickey's built subtree. */
export class Instance extends transformable(SceneValue) {
  constructor(ref, matrix = null, color = null, label = null) {
    super(matrix, color, label);
    this._ref = ref; // content hash of the built subtree
  }

  _with({ matrix = this._matrix, color = this._color, name = this._name }) {
    return new Instance(this._ref, matrix, color, name);
  }

  _toIR() {
    if (this._name === null && isIdentity(this._matrix) && this._color === null) {
      return { ref: this._ref };
    }
    return {
      ...(this._name !== null && { name: this._name }),
      ...(!isIdentity(this._matrix) && { matrix: [...matElements(this._matrix)] }),
      ...(this._color !== null && { color: this._color }),
      children: [{ ref: this._ref }],
    };
  }
}

function toIRNode(v) {
  if (v === null || v === undefined) return {};
  if (v instanceof Solid || v instanceof Group || v instanceof Instance) return v._toIR();
  if (Array.isArray(v)) {
    return { children: v.filter((c) => c !== null && c !== undefined).map(toIRNode) };
  }
  if (v instanceof THREE.BufferGeometry) return fromThreeGeometry(v)._toIR();
  throw new TypeError(
    `cannot put a ${v?.constructor?.name ?? typeof v} in the scene: ` +
      'build() must return Solids, Groups, Instances, three.js geometries, arrays of those, or null',
  );
}

// ---------- primitives ----------

function num(v, what) {
  if (typeof v !== 'number' || !Number.isFinite(v)) {
    throw new TypeError(`${what} must be a finite number, got ${v}`);
  }
  return v;
}

/** Box. `odm.box(10)`, `odm.box([x,y,z])`, or `odm.box({size, center})`. Centered by default. */
export function box(size, opts = {}) {
  let s = size;
  let center = opts.center ?? true;
  if (size && typeof size === 'object' && !Array.isArray(size)) {
    s = size.size;
    center = size.center ?? true;
  }
  if (typeof s === 'number') s = [s, s, s];
  if (!Array.isArray(s) || s.length !== 3) {
    throw new TypeError('box size must be a number or [x, y, z]');
  }
  s.forEach((v) => num(v, 'box size'));
  return new Solid(ops().op_solid_box(s, center));
}

/**
 * Cylinder along Z. `odm.cylinder(r, h)` or
 * `odm.cylinder({ r | r1, r2, h, segments, center })`. Centered by default;
 * `center: false` puts the base at z=0.
 */
export function cylinder(a, b, opts = {}) {
  let o;
  if (typeof a === 'object') {
    o = a;
  } else {
    o = { ...opts, r: a, h: b };
  }
  const r1 = num(o.r1 ?? o.r, 'cylinder radius');
  const r2 = o.r2 ?? o.r1 ?? o.r;
  const h = num(o.h ?? o.height, 'cylinder height');
  return new Solid(ops().op_solid_cylinder(h, r1, num(r2, 'cylinder r2'), o.segments ?? 64, o.center ?? true));
}

/** Sphere at the origin. `odm.sphere(r)` or `odm.sphere({ r, segments })`. */
export function sphere(a, opts = {}) {
  const o = typeof a === 'object' ? a : { ...opts, r: a };
  return new Solid(ops().op_solid_sphere(num(o.r ?? o.radius, 'sphere radius'), o.segments ?? 48));
}

// ---------- 2D profiles → solids ----------

function pt2(p, what) {
  if (Array.isArray(p) && p.length === 2) return [num(p[0], what), num(p[1], what)];
  if (p && typeof p.x === 'number' && typeof p.y === 'number') return [p.x, p.y];
  throw new TypeError(`${what} must be [x, y] pairs or Vector2s`);
}

/**
 * Profile → list of polygons. Accepts a THREE.Shape (with holes; curves are
 * flattened with `curveSegments`), one polygon `[[x,y], ...]`, or a list of
 * polygons (first outer, rest holes — or any even-odd arrangement).
 */
function toPolygons(profile, curveSegments = 32) {
  if (profile instanceof THREE.Shape) {
    const pts = profile.extractPoints(curveSegments);
    return [pts.shape.map((p) => [p.x, p.y]), ...pts.holes.map((h) => h.map((p) => [p.x, p.y]))];
  }
  if (profile instanceof THREE.Path) {
    return [profile.getPoints(curveSegments).map((p) => [p.x, p.y])];
  }
  if (Array.isArray(profile) && profile.length > 0) {
    const first = profile[0];
    if (Array.isArray(first) && first.length === 2 && typeof first[0] === 'number') {
      return [profile.map((p) => pt2(p, 'profile point'))];
    }
    if (first && typeof first.x === 'number') {
      return [profile.map((p) => pt2(p, 'profile point'))];
    }
    return profile.map((poly) => poly.map((p) => pt2(p, 'profile point')));
  }
  throw new TypeError('profile must be a THREE.Shape, [[x,y], ...], or a list of polygons');
}

/**
 * Extrude a 2D profile along +Z.
 * `odm.extrude(profile, { height, twist = 0, scale = 1, slices, curveSegments })`
 * — twist in radians over the full height; scale is the top scale factor
 * (number or [x, y]).
 */
export function extrude(profile, opts = {}) {
  const height = num(opts.height ?? opts.depth ?? opts.h, 'extrude height');
  const twist = opts.twist ?? 0;
  let scale = opts.scale ?? 1;
  if (typeof scale === 'number') scale = [scale, scale];
  const twistDeg = (twist * 180) / Math.PI;
  const slices = opts.slices ?? (twist !== 0 ? Math.max(2, Math.ceil(Math.abs(twistDeg) / 10)) : 1);
  const polys = toPolygons(profile, opts.curveSegments);
  return new Solid(ops().op_solid_extrude(polys, height, slices, twistDeg, scale));
}

/**
 * Revolve a 2D profile (x >= 0) around the Z axis; profile (x, y) maps to
 * (radius, z). `odm.revolve(profile, { angle = 2π, segments = 64 })`.
 */
export function revolve(profile, opts = {}) {
  const angle = opts.angle ?? Math.PI * 2;
  const polys = toPolygons(profile, opts.curveSegments);
  return new Solid(ops().op_solid_revolve(polys, opts.segments ?? 64, (angle * 180) / Math.PI));
}

// ---------- three.js interop ----------

/**
 * Turn a closed three.js BufferGeometry (BoxGeometry, TorusGeometry,
 * ExtrudeGeometry, closed LatheGeometry, ...) into a Solid. The mesh is
 * welded engine-side; open surfaces are rejected with an explanation.
 */
export function fromThreeGeometry(g) {
  if (!(g instanceof THREE.BufferGeometry)) {
    throw new TypeError('fromThreeGeometry takes a THREE.BufferGeometry');
  }
  const posAttr = g.getAttribute('position');
  if (!posAttr) throw new TypeError('geometry has no position attribute');
  const positions = posAttr.array instanceof Float32Array ? posAttr.array : Float32Array.from(posAttr.array);
  let indices;
  if (g.index) {
    indices = g.index.array instanceof Uint32Array ? g.index.array : Uint32Array.from(g.index.array);
  } else {
    indices = new Uint32Array(posAttr.count);
    for (let i = 0; i < indices.length; i++) indices[i] = i;
  }
  return new Solid(ops().op_solid_from_mesh(positions, indices));
}

// ---------- misc API ----------

export function group(...children) {
  const kids = children.flat().filter((c) => c !== null && c !== undefined);
  for (const c of kids) {
    if (
      !(c instanceof Solid || c instanceof Group || c instanceof Instance || Array.isArray(c) ||
        c instanceof THREE.BufferGeometry)
    ) {
      throw new TypeError(`group() child must be a scene value, got ${c?.constructor?.name ?? typeof c}`);
    }
  }
  return new Group(kids);
}

export function union(first, ...rest) {
  if (!(first instanceof Solid)) throw new TypeError('union takes Solids');
  return first.union(...rest);
}
export function difference(first, ...rest) {
  if (!(first instanceof Solid)) throw new TypeError('difference takes Solids');
  return first.subtract(...rest);
}
export function intersection(first, ...rest) {
  if (!(first instanceof Solid)) throw new TypeError('intersection takes Solids');
  return first.intersect(...rest);
}
export function hull(first, ...rest) {
  if (!(first instanceof Solid)) throw new TypeError('hull takes Solids');
  return first.hull(...rest);
}

/** Degrees → radians. */
export function deg(d) {
  return (d * Math.PI) / 180;
}

// ---------- args serialization (Solids cross invoke() by content hash) ----------

const SOLID_TAG = '__odm_solid__';

// THREE instances normalize to canonical wire JSON at every boundary
// (invoke args, provides): vectors/quaternions via toArray, matrix4 = 16
// numbers column-major, colors [r, g, b]. Hashing and memoization only ever
// see the canonical form.
function serializeValue(v) {
  if (v === undefined) return null;
  return JSON.parse(
    JSON.stringify(v, function (key, value) {
      const raw = this[key];
      if (raw instanceof Solid) {
        return {
          [SOLID_TAG]: true,
          geom: raw._geom,
          matrix: isIdentity(raw._matrix) ? null : [...matElements(raw._matrix)],
          color: raw._color,
          name: raw._name,
        };
      }
      if (raw instanceof Group || raw instanceof Instance) {
        throw new TypeError('only Solids (not Groups/Instances) can be passed through invoke() args');
      }
      if (
        raw instanceof THREE.Vector2 ||
        raw instanceof THREE.Vector3 ||
        raw instanceof THREE.Quaternion
      ) {
        return raw.toArray();
      }
      if (raw instanceof THREE.Matrix4) {
        return raw.toArray();
      }
      return value;
    }),
  );
}

function reviveValue(v) {
  if (v && typeof v === 'object') {
    if (v[SOLID_TAG]) {
      return new Solid(
        v.geom,
        v.matrix ? new THREE.Matrix4().fromArray(v.matrix) : null,
        v.color ?? null,
        v.name ?? null,
      );
    }
    if (Array.isArray(v)) return v.map(reviveValue);
    const out = {};
    for (const [k, val] of Object.entries(v)) out[k] = reviveValue(val);
    return out;
  }
  return v;
}

// ---------- build context ----------

// Declaration-driven hydration: the wire carries canonical JSON; ctx.get
// returns real THREE instances for the extension types. Colors stay in
// their wire form (hex string or [r, g, b]) — exactly what .color() takes.
function hydrate(type, v) {
  const nums = (v, n) => {
    if (Array.isArray(v) && v.length === n) return v;
    // Tolerate the {x, y, z} object form (e.g. hand-written provides).
    if (v && typeof v === 'object') {
      const parts = ['x', 'y', 'z', 'w'].slice(0, n).map((k) => v[k]);
      if (parts.every((p) => typeof p === 'number')) return parts;
    }
    throw new TypeError(`expected ${n} numbers for a ${type}, got ${JSON.stringify(v)}`);
  };
  switch (type) {
    case 'vector2':
      return new THREE.Vector2(...nums(v, 2));
    case 'vector3':
      return new THREE.Vector3(...nums(v, 3));
    case 'quaternion':
      return new THREE.Quaternion(...nums(v, 4));
    case 'matrix4':
      return new THREE.Matrix4().fromArray(Array.isArray(v) ? v : v.elements);
    default:
      // 'solid' arrives as a tagged handle that reviveValue turns back into
      // a Solid; plain JSON values may carry nested Solids too.
      return reviveValue(v);
  }
}

function makeCtx(argsJson, decls) {
  const args = argsJson ?? {};
  return {
    /**
     * Read one declared input (see `export const meta`). Plain inputs come
     * from the immediate caller's args (or the declared default); cascade
     * inputs resolve up the invoke chain, view outermost.
     */
    get(name) {
      name = String(name);
      const decl = decls[name];
      if (!decl) {
        const known = Object.keys(decls);
        throw new Error(
          `ctx.get(${JSON.stringify(name)}): not declared in meta.inputs` +
            (known.length ? ` (declared: ${known.join(', ')})` : ' (this file declares no inputs)'),
        );
      }
      let raw;
      if (decl.cascade) {
        const r = ops().op_context_read(name);
        if (!r.present) {
          throw new Error(`internal: cascade input ${JSON.stringify(name)} missing from environment`);
        }
        raw = r.value;
      } else {
        raw = args[name];
      }
      return hydrate(decl.type, raw);
    },
    /**
     * Build another doohickey and get its output as an Instance.
     * `path` is project-relative, e.g. 'parts/wheel.js'. `args` go to that
     * file's declared inputs; `provides` scope over its whole subtree
     * (cascade values, no declaration needed here).
     */
    invoke(path, args = {}, provides = {}) {
      return new Instance(
        ops().op_invoke(String(path), serializeValue(args), serializeValue(provides)),
      );
    },
  };
}

// ---------- engine plumbing ----------

function safeString(v) {
  if (typeof v === 'string') return v;
  try {
    return JSON.stringify(v) ?? String(v);
  } catch {
    return String(v);
  }
}

export function installGlobals(g) {
  g.odm = {
    Solid,
    Group,
    Instance,
    box,
    cylinder,
    sphere,
    extrude,
    revolve,
    fromThreeGeometry,
    group,
    union,
    difference,
    intersection,
    hull,
    deg,
    parseColor,
  };

  const log = (level) => (...a) => {
    const o = globalThis.Deno?.core?.ops;
    if (o?.op_log) o.op_log(level, a.map(safeString).join(' '));
  };
  g.console = {
    log: log('log'),
    info: log('log'),
    debug: log('debug'),
    warn: log('warn'),
    error: log('error'),
  };

  g.__odm = {
    runBuild(ns, argsJson, declsJson) {
      const fn = ns?.default;
      if (typeof fn !== 'function') {
        throw new TypeError(
          'doohickey must have a default export: `export default function build(ctx) { ... }`',
        );
      }
      return toIRNode(fn(makeCtx(argsJson ?? {}, declsJson ?? {})));
    },
  };
}
