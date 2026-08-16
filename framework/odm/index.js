// ODM framework: the API doohickeys build with.
//
// Conventions: Z-up, distances in project units, ALL angles in radians
// (like three.js; `odm.deg(90)` converts). Scene values are immutable —
// every method returns a new value; a call whose result you discard does
// nothing. Geometry lives engine-side; a Solid holds an opaque
// content-hash handle plus a pending transform and color.
import * as THREE from '../three/entry.js';
import { parseColor } from './colors.js';

function ops() {
  const o = globalThis.Deno?.core?.ops;
  if (!o || !o.op_solid_box) {
    throw new Error('ODM engine ops unavailable: this code only runs inside a build');
  }
  return o;
}

// ---------- argument checking ----------

function num(v, what) {
  if (typeof v !== 'number' || !Number.isFinite(v)) {
    throw new TypeError(`${what} must be a finite number, got ${v}`);
  }
  return v;
}

/** Options objects reject unknown keys: a typo'd option must not silently no-op. */
function checkOpts(opts, allowed, what) {
  if (opts === undefined) return {};
  if (opts === null || typeof opts !== 'object' || Array.isArray(opts)) {
    throw new TypeError(`${what} options must be an object, got ${opts === null ? 'null' : typeof opts}`);
  }
  for (const k of Object.keys(opts)) {
    if (!allowed.includes(k)) {
      throw new TypeError(`unknown ${what} option '${k}' (valid: ${allowed.join(', ')})`);
    }
  }
  return opts;
}

function vec3(v, what) {
  if (Array.isArray(v) && v.length === 3) {
    return [num(v[0], what), num(v[1], what), num(v[2], what)];
  }
  if (v instanceof THREE.Vector3) return [v.x, v.y, v.z];
  throw new TypeError(`${what} must be [x, y, z] or a THREE.Vector3`);
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
  throw new TypeError('applyMatrix4() takes a THREE.Matrix4 or a column-major array of 16 numbers');
}

/** Left-multiply: apply `extra` (in world frame) after the existing matrix. */
function premul(matrix, extra) {
  const base = matrix === null ? new THREE.Matrix4() : matrix;
  return extra.clone().multiply(base);
}

// ---------- scene values: Solid, Group, Instance ----------

const transformable = (Base) =>
  class extends Base {
    /** Apply `m`, conjugated by translation to `opts.about` if given. */
    _pivoted(m, opts, what) {
      const o = checkOpts(opts, ['about'], what);
      let full = m;
      if (o.about !== undefined) {
        const [cx, cy, cz] = vec3(o.about, `${what} about`);
        full = new THREE.Matrix4()
          .makeTranslation(cx, cy, cz)
          .multiply(m)
          .multiply(new THREE.Matrix4().makeTranslation(-cx, -cy, -cz));
      }
      return this._with({ matrix: premul(this._matrix, full) });
    }
    translate(x, y, z) {
      return this._with({
        matrix: premul(
          this._matrix,
          new THREE.Matrix4().makeTranslation(
            num(x, 'translate x'),
            num(y, 'translate y'),
            num(z, 'translate z'),
          ),
        ),
      });
    }
    rotateX(rad, opts) {
      return this._pivoted(new THREE.Matrix4().makeRotationX(num(rad, 'rotateX angle')), opts, 'rotateX');
    }
    rotateY(rad, opts) {
      return this._pivoted(new THREE.Matrix4().makeRotationY(num(rad, 'rotateY angle')), opts, 'rotateY');
    }
    rotateZ(rad, opts) {
      return this._pivoted(new THREE.Matrix4().makeRotationZ(num(rad, 'rotateZ angle')), opts, 'rotateZ');
    }
    /** Rotate around an arbitrary axis (normalized for you). */
    rotate(axis, rad, opts) {
      const a = new THREE.Vector3(...vec3(axis, 'rotate axis'));
      if (a.lengthSq() === 0) throw new TypeError('rotate axis must be nonzero');
      return this._pivoted(
        new THREE.Matrix4().makeRotationAxis(a.normalize(), num(rad, 'rotate angle')),
        opts,
        'rotate',
      );
    }
    scale(x, y, z, opts) {
      if (y === undefined || z === undefined) {
        throw new TypeError('scale(x, y, z) takes all three factors (uniform: scale(k, k, k))');
      }
      return this._pivoted(
        new THREE.Matrix4().makeScale(num(x, 'scale x'), num(y, 'scale y'), num(z, 'scale z')),
        opts,
        'scale',
      );
    }
    applyMatrix4(m) {
      return this._with({ matrix: premul(this._matrix, toMatrix4(m)) });
    }
    color(c) {
      return this._with({ color: parseColor(c) });
    }
    /**
     * Multiply this subtree's opacity by `x` (0..1). Unlike color (which
     * children override), opacity is multiplicative down the tree: a 50%
     * group shows its internals through each other — x-ray, not flattening.
     */
    opacity(x) {
      const v = num(x, 'opacity');
      if (v < 0 || v > 1) throw new TypeError(`opacity must be in 0..1, got ${v}`);
      const prev = this._opacity ?? 1;
      return this._with({ opacity: prev * v });
    }
    name(n) {
      return this._with({ name: String(n) });
    }
  };

class SceneValue {
  constructor(matrix, color, label, opacity) {
    this._matrix = matrix; // THREE.Matrix4 | null (identity)
    this._color = color; // linear [r,g,b,a] | null
    this._name = label; // string | null
    this._opacity = opacity ?? null; // number | null (1)
  }
}

/** A solid body: an engine-side geometry handle + pending transform/color. */
export class Solid extends transformable(SceneValue) {
  constructor(geom, matrix = null, color = null, label = null, opacity = null) {
    super(matrix, color, label, opacity);
    this._geom = geom; // content-hash hex string
    this._bakedCache = null;
  }

  _with({
    matrix = this._matrix,
    color = this._color,
    name = this._name,
    opacity = this._opacity,
  }) {
    return new Solid(this._geom, matrix, color, name, opacity);
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
  subtract(...others) {
    return Solid._bool('difference', [this, ...others.flat()], this);
  }
  intersect(...others) {
    return Solid._bool('intersection', [this, ...others.flat()], this);
  }
  hull(...others) {
    const all = [this, ...others.flat()];
    for (const s of all) {
      if (!(s instanceof Solid)) {
        throw new TypeError(
          `hull operands must be Solids (got ${s?.constructor?.name ?? typeof s}); ` +
            'Groups/Instances cannot be used in CSG',
        );
      }
    }
    const geom = ops().op_hull(all.map((s) => s._operand()));
    return new Solid(geom, null, this._color, this._name);
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
  /** Axis-aligned bounds in this solid's (world) frame: THREE.Box3, or null if empty. */
  bounds() {
    const b = ops().op_bounds(this._baked());
    if (!b) return null;
    return new THREE.Box3(new THREE.Vector3(...b.min), new THREE.Vector3(...b.max));
  }
  /**
   * Nearest surface hit of the ray from `origin` along `dir` (each [x,y,z]
   * or Vector3), or null. Returns { distance, point: Vector3, normal: Vector3 }.
   */
  raycast(origin, dir, maxDist = 1e9) {
    const hit = ops().op_raycast(
      this._baked(),
      vec3(origin, 'raycast origin'),
      vec3(dir, 'raycast dir'),
      num(maxDist, 'raycast maxDist'),
    );
    if (!hit) return null;
    return {
      distance: hit.distance,
      point: new THREE.Vector3(...hit.position),
      normal: new THREE.Vector3(...hit.normal),
    };
  }

  _toIR() {
    return {
      ...(this._name !== null && { name: this._name }),
      ...(!isIdentity(this._matrix) && { matrix: [...matElements(this._matrix)] }),
      ...(this._color !== null && { color: this._color }),
      ...(this._opacity !== null && { opacity: this._opacity }),
      geom: this._geom,
    };
  }
}

/** A pure grouping of scene values under one transform/color. */
export class Group extends transformable(SceneValue) {
  constructor(children, matrix = null, color = null, label = null, opacity = null) {
    super(matrix, color, label, opacity);
    this._children = children;
  }

  _with({
    matrix = this._matrix,
    color = this._color,
    name = this._name,
    opacity = this._opacity,
  }) {
    return new Group(this._children, matrix, color, name, opacity);
  }

  get children() {
    return [...this._children];
  }

  _toIR() {
    return {
      ...(this._name !== null && { name: this._name }),
      ...(!isIdentity(this._matrix) && { matrix: [...matElements(this._matrix)] }),
      ...(this._color !== null && { color: this._color }),
      ...(this._opacity !== null && { opacity: this._opacity }),
      children: this._children.map(toIRNode),
    };
  }
}

/** The output of ctx.invoke(): another doohickey's built subtree. */
export class Instance extends transformable(SceneValue) {
  constructor(ref, matrix = null, color = null, label = null, opacity = null) {
    super(matrix, color, label, opacity);
    this._ref = ref; // content hash of the built subtree
  }

  _with({
    matrix = this._matrix,
    color = this._color,
    name = this._name,
    opacity = this._opacity,
  }) {
    return new Instance(this._ref, matrix, color, name, opacity);
  }

  _toIR() {
    if (
      this._name === null &&
      isIdentity(this._matrix) &&
      this._color === null &&
      this._opacity === null
    ) {
      return { ref: this._ref };
    }
    return {
      ...(this._name !== null && { name: this._name }),
      ...(!isIdentity(this._matrix) && { matrix: [...matElements(this._matrix)] }),
      ...(this._color !== null && { color: this._color }),
      ...(this._opacity !== null && { opacity: this._opacity }),
      children: [{ ref: this._ref }],
    };
  }
}

function sceneTypeError(v, where) {
  if (v instanceof THREE.BufferGeometry) {
    return new TypeError(
      `${where}: raw three.js geometry is not a scene value — wrap it with odm.fromThreeGeometry() first`,
    );
  }
  return new TypeError(
    `${where}: cannot use a ${v?.constructor?.name ?? typeof v} — ` +
      'scene values are Solids, Groups, Instances, arrays of those, or null',
  );
}

function toIRNode(v) {
  if (v === null || v === undefined) return {};
  if (v instanceof Solid || v instanceof Group || v instanceof Instance) return v._toIR();
  if (Array.isArray(v)) {
    return { children: v.filter((c) => c !== null && c !== undefined).map(toIRNode) };
  }
  throw sceneTypeError(v, 'build() return value');
}

// ---------- primitives ----------

/** Box. `odm.box(10)` (cube) or `odm.box([x, y, z])`. Centered by default. */
export function box(size, opts) {
  const o = checkOpts(opts, ['center'], 'box');
  let s = typeof size === 'number' ? [size, size, size] : size;
  if (!Array.isArray(s) || s.length !== 3) {
    throw new TypeError('box size must be a number or [x, y, z]');
  }
  s = s.map((v) => num(v, 'box size'));
  return new Solid(ops().op_solid_box(s, o.center ?? true));
}

/**
 * Cylinder along Z. `odm.cylinder(r, h, { r2, segments, center })` — `r2`
 * is the top radius (cone; defaults to `r`). Centered by default;
 * `center: false` puts the base at z=0.
 */
export function cylinder(r, h, opts) {
  const o = checkOpts(opts, ['r2', 'segments', 'center'], 'cylinder');
  const r1 = num(r, 'cylinder radius');
  const r2 = o.r2 === undefined ? r1 : num(o.r2, 'cylinder r2');
  const segments = o.segments === undefined ? 64 : num(o.segments, 'cylinder segments');
  return new Solid(ops().op_solid_cylinder(num(h, 'cylinder height'), r1, r2, segments, o.center ?? true));
}

/** Sphere at the origin. `odm.sphere(r, { segments })`. */
export function sphere(r, opts) {
  const o = checkOpts(opts, ['segments'], 'sphere');
  const segments = o.segments === undefined ? 48 : num(o.segments, 'sphere segments');
  return new Solid(ops().op_solid_sphere(num(r, 'sphere radius'), segments));
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
function toPolygons(profile, curveSegments) {
  const cs = curveSegments === undefined ? 32 : num(curveSegments, 'curveSegments');
  if (profile instanceof THREE.Shape) {
    const pts = profile.extractPoints(cs);
    return [pts.shape.map((p) => [p.x, p.y]), ...pts.holes.map((h) => h.map((p) => [p.x, p.y]))];
  }
  if (profile instanceof THREE.Path) {
    return [profile.getPoints(cs).map((p) => [p.x, p.y])];
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
 * Extrude a 2D profile along +Z, from z=0 to z=height.
 * `odm.extrude(profile, height, { twist = 0, scale = 1, slices, curveSegments })`
 * — twist in radians over the full height; scale is the top scale factor
 * (number or [x, y]).
 */
export function extrude(profile, height, opts) {
  const o = checkOpts(opts, ['twist', 'scale', 'slices', 'curveSegments'], 'extrude');
  const h = num(height, 'extrude height');
  const twist = o.twist === undefined ? 0 : num(o.twist, 'extrude twist');
  let scale = o.scale ?? 1;
  if (typeof scale === 'number') scale = [scale, scale];
  if (!Array.isArray(scale) || scale.length !== 2) {
    throw new TypeError('extrude scale must be a number or [x, y]');
  }
  scale = scale.map((v) => num(v, 'extrude scale'));
  const twistDeg = (twist * 180) / Math.PI;
  const slices =
    o.slices === undefined
      ? twist !== 0
        ? Math.max(2, Math.ceil(Math.abs(twistDeg) / 10))
        : 1
      : num(o.slices, 'extrude slices');
  const polys = toPolygons(profile, o.curveSegments);
  return new Solid(ops().op_solid_extrude(polys, h, slices, twistDeg, scale));
}

/**
 * Revolve a 2D profile (x >= 0) around the Z axis; profile (x, y) maps to
 * (radius, z). `odm.revolve(profile, { angle = 2π, segments = 64 })`.
 */
export function revolve(profile, opts) {
  const o = checkOpts(opts, ['angle', 'segments', 'curveSegments'], 'revolve');
  const angle = o.angle === undefined ? Math.PI * 2 : num(o.angle, 'revolve angle');
  const segments = o.segments === undefined ? 64 : num(o.segments, 'revolve segments');
  const polys = toPolygons(profile, o.curveSegments);
  return new Solid(ops().op_solid_revolve(polys, segments, (angle * 180) / Math.PI));
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
    if (!(c instanceof Solid || c instanceof Group || c instanceof Instance || Array.isArray(c))) {
      throw sceneTypeError(c, 'group() child');
    }
  }
  return new Group(kids);
}

/** Degrees → radians. */
export function deg(d) {
  return (num(d, 'deg') * Math.PI) / 180;
}

// ---------- args serialization (Solids cross invoke() by content hash) ----------

const SOLID_TAG = '__odm_solid__';

// THREE instances normalize to canonical wire JSON at every boundary
// (invoke args and cascade): vectors/quaternions via toArray, matrix4 = 16
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
          opacity: raw._opacity,
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
        v.opacity ?? null,
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

// Declaration-driven hydration: the wire carries canonical JSON; ctx.input
// returns real THREE instances for the extension types. Colors stay in
// their wire form (hex string or [r, g, b]) — exactly what .color() takes.
function hydrate(type, v) {
  const nums = (v, n) => {
    if (Array.isArray(v) && v.length === n) return v;
    // Tolerate the {x, y, z} object form (e.g. hand-written cascade values).
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
    input(name) {
      name = String(name);
      const decl = decls[name];
      if (!decl) {
        const known = Object.keys(decls);
        throw new Error(
          `ctx.input(${JSON.stringify(name)}): not declared in meta.inputs` +
            (known.length ? ` (declared: ${known.join(', ')})` : ' (this file declares no inputs)'),
        );
      }
      let raw;
      if (decl.cascade) {
        const r = ops().op_cascade_read(name);
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
     * file's declared inputs; `cascade` values scope over its whole
     * subtree (no declaration needed here).
     */
    invoke(path, args = {}, cascade = {}) {
      return new Instance(
        ops().op_invoke(String(path), serializeValue(args), serializeValue(cascade)),
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
    deg,
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
