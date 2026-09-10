// ODM framework: the API doohickeys build with.
//
// Conventions: Z-up, distances in project units, ALL angles in radians
// (like three.js; `odm.deg(90)` converts). Scene values are immutable —
// every method returns a new value; a call whose result you discard does
// nothing. Geometry lives engine-side; a Solid holds an opaque
// content-hash handle plus a pending transform and color.
import * as THREE from '../three/entry.js';
import { parseColor } from './colors.js';

// The engine-ops seam: the host (V8 isolate natively, the web export's
// runtime in a browser) supplies one object with every op_* function.
function ops() {
  const o = globalThis.__odmOps ?? globalThis.Deno?.core?.ops;
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

/**
 * A dimension that must be strictly positive. Manifold answers a zero or
 * negative size with a bare InvalidConstruction status, which says nothing
 * about which argument was wrong.
 */
function pos(v, what) {
  const n = num(v, what);
  if (n <= 0) throw new RangeError(`${what} must be positive, got ${n}`);
  return n;
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
    return new Solid(geom, null, keep._color, keep._name, keep._opacity);
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
    return new Solid(geom, null, this._color, this._name, this._opacity);
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

  /**
   * Signed distance to another Solid (both in their current frames).
   * Positive: the exact minimum gap, with `closest: [Vector3, Vector3]`
   * (points on this solid and on `other`). Negative: the solids overlap;
   * `-distance` is the length of `separate: Vector3` — translate `other`
   * by it to clear this solid (a guaranteed separation, an upper bound on
   * true penetration depth). The sign of a near-zero value is float noise
   * (exact tangency): treat `|distance|` below your own tolerance as
   * contact — don't nudge geometry to disambiguate.
   */
  clearance(other) {
    if (!(other instanceof Solid)) {
      throw new TypeError(
        `clearance takes a Solid (got ${other?.constructor?.name ?? typeof other})`,
      );
    }
    const c = ops().op_clearance(this._baked(), other._baked());
    const out = { distance: c.distance };
    if (c.closest) out.closest = c.closest.map((p) => new THREE.Vector3(...p));
    if (c.separate) out.separate = new THREE.Vector3(...c.separate);
    return out;
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
  s = s.map((v) => pos(v, 'box size'));
  return new Solid(ops().op_solid_box(s, o.center ?? true));
}

/**
 * Cylinder along Z. `odm.cylinder(r, h, { r2, segments, center })` — `r2`
 * is the top radius (cone; defaults to `r`). Centered by default;
 * `center: false` puts the base at z=0.
 */
export function cylinder(r, h, opts) {
  const o = checkOpts(opts, ['r2', 'segments', 'center'], 'cylinder');
  const r1 = pos(r, 'cylinder radius');
  // r2 = 0 is the cone tip, so only negatives are out.
  const r2 = o.r2 === undefined ? r1 : num(o.r2, 'cylinder r2');
  if (r2 < 0) throw new RangeError(`cylinder r2 must not be negative, got ${r2}`);
  const segments = o.segments === undefined ? 64 : num(o.segments, 'cylinder segments');
  return new Solid(ops().op_solid_cylinder(pos(h, 'cylinder height'), r1, r2, segments, o.center ?? true));
}

/** Sphere at the origin. `odm.sphere(r, { segments })`. */
export function sphere(r, opts) {
  const o = checkOpts(opts, ['segments'], 'sphere');
  const segments = o.segments === undefined ? 48 : num(o.segments, 'sphere segments');
  return new Solid(ops().op_solid_sphere(pos(r, 'sphere radius'), segments));
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
 * polygons (first outer, rest holes — each hole wound the opposite way, which
 * is what the kernel's Positive fill rule means by a hole).
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
  const h = pos(height, 'extrude height');
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
  // x is a radius here. A negative one folds the profile through the axis
  // and yields quietly wrong geometry, so it is an error (errors.md).
  for (const poly of polys) {
    for (const [x] of poly) {
      if (x < 0) throw new RangeError(`revolve profile x must be >= 0 (it is a radius), got ${x}`);
    }
  }
  return new Solid(ops().op_solid_revolve(polys, segments, (angle * 180) / Math.PI));
}

/** A sweep path point: [x, y, z], [x, y], Vector3 or Vector2 (z defaults to 0). */
function pathPoint(p) {
  const what = 'sweep path point';
  if (Array.isArray(p) && (p.length === 2 || p.length === 3)) {
    return new THREE.Vector3(num(p[0], what), num(p[1], what), p.length === 3 ? num(p[2], what) : 0);
  }
  if (p && typeof p.x === 'number' && typeof p.y === 'number') {
    return new THREE.Vector3(p.x, p.y, typeof p.z === 'number' ? p.z : 0);
  }
  throw new TypeError(`${what} must be [x, y, z] or a THREE.Vector3`);
}

/** Path → distinct station points. A THREE.Curve is sampled; an array is used as given. */
function pathStations(path, o) {
  let raw;
  if (path instanceof THREE.Curve) {
    const segments = o.segments === undefined ? 64 : num(o.segments, 'sweep segments');
    if (!Number.isInteger(segments) || segments < 1) {
      throw new RangeError(`sweep segments must be an integer >= 1, got ${segments}`);
    }
    raw = path.getSpacedPoints(segments);
  } else if (Array.isArray(path)) {
    if (o.segments !== undefined) {
      throw new TypeError(
        'sweep segments only applies to a THREE.Curve path; a point array is swept as given',
      );
    }
    raw = path;
  } else {
    throw new TypeError('sweep path must be an array of points or a THREE.Curve');
  }
  const pts = [];
  for (const p of raw) {
    const v = pathPoint(p);
    if (pts.length === 0 || pts[pts.length - 1].distanceTo(v) > 0) pts.push(v);
  }
  if (pts.length < 2) {
    throw new TypeError(`sweep path needs 2 or more distinct points, got ${pts.length}`);
  }
  return pts;
}

/** Sharper than this and the miter blows up; the agent should add points. */
const MAX_SWEEP_TURN = (150 * Math.PI) / 180;

/**
 * Rotation-minimizing frames along the station points: one row-major 3x4
 * affine per station, mapping profile (x, y) into place. Right-handed with
 * x cross y along the direction of travel, as the kernel requires.
 */
function sweepFrames(pts, o, reach) {
  const n = pts.length;
  const seg = [];
  for (let i = 0; i < n - 1; i++) seg.push(pts[i + 1].clone().sub(pts[i]).normalize());

  // Station tangents: the segment tangent at the ends, the bisector inside.
  const tangent = [seg[0]];
  const turn = [0];
  for (let i = 1; i < n - 1; i++) {
    const angle = Math.acos(Math.min(1, Math.max(-1, seg[i - 1].dot(seg[i]))));
    if (angle > MAX_SWEEP_TURN) {
      throw new RangeError(
        `sweep path turns ${Math.round((angle * 180) / Math.PI)}° at point ${i}; ` +
          'the miter is only defined below 150° — add intermediate points or use a curve',
      );
    }
    turn.push(angle);
    tangent.push(seg[i - 1].clone().add(seg[i]).normalize());
  }
  tangent.push(seg[n - 2]);
  turn.push(0);
  warnTightBends(pts, turn, reach);

  // Profile +y follows `up`, projected perpendicular to the first tangent.
  const auto = o.up === undefined;
  const up = auto ? new THREE.Vector3(0, 0, 1) : new THREE.Vector3(...vec3(o.up, 'sweep up'));
  if (up.lengthSq() === 0) throw new TypeError('sweep up must not be the zero vector');
  up.normalize();
  // Z-up default: a horizontal path gets a flat ribbon. Vertical paths fall
  // back to +Y, where +Z would be degenerate.
  if (auto && Math.abs(up.dot(tangent[0])) > Math.cos((1 * Math.PI) / 180)) up.set(0, 1, 0);
  const carried = up.clone().addScaledVector(tangent[0], -up.dot(tangent[0]));
  if (carried.length() < 1e-9) {
    throw new Error("sweep up is parallel to the path's first segment; pass a different up");
  }
  carried.normalize();

  const frames = [];
  for (let i = 0; i < n; i++) {
    if (i > 0) {
      // Parallel transport: the minimal rotation taking the previous station's
      // tangent to this one carries the frame with no extra spin.
      carried.applyQuaternion(new THREE.Quaternion().setFromUnitVectors(tangent[i - 1], tangent[i]));
      carried.addScaledVector(tangent[i], -carried.dot(tangent[i])).normalize();
    }
    const ay = carried.clone();
    const ax = ay.clone().cross(tangent[i]);
    if (turn[i] > 0) {
      // Miter: the profile sits in the bisector plane, stretched by
      // 1/cos(turn/2) along the in-bend direction, so walls keep their
      // thickness through the corner.
      const d = seg[i].clone().sub(seg[i - 1]).normalize();
      const s = 1 / Math.cos(turn[i] / 2) - 1;
      ax.addScaledVector(d, s * d.dot(ax));
      ay.addScaledVector(d, s * d.dot(ay));
    }
    const p = pts[i];
    const t = tangent[i];
    frames.push([ax.x, ay.x, t.x, p.x, ax.y, ay.y, t.y, p.y, ax.z, ay.z, t.z, p.z]);
  }
  return frames;
}

/**
 * A bend tighter than the profile is wide folds the solid through itself:
 * Manifold accepts the mesh but volume and booleans go wrong. Warn (once) —
 * a slightly overlapping cable usually still looks right.
 */
function warnTightBends(pts, turn, reach) {
  for (let i = 1; i < pts.length - 1; i++) {
    if (turn[i] <= 0) continue;
    const legs = Math.min(pts[i].distanceTo(pts[i - 1]), pts[i].distanceTo(pts[i + 1]));
    const radius = legs / 2 / Math.tan(turn[i] / 2);
    if (radius < reach) {
      console.warn(
        `sweep: the bend at path point ${i} has radius ~${radius.toPrecision(3)}, tighter than ` +
          `the profile's ${reach.toPrecision(3)} reach — the solid self-intersects there, ` +
          'so volume and CSG will be wrong',
      );
      return;
    }
  }
}

/**
 * Sweep a 2D profile along a 3D path.
 * `odm.sweep(profile, path, { segments = 64, up, curveSegments })` — `path`
 * is a point array (a polyline, used as given) or a `THREE.Curve` (sampled
 * into `segments` pieces). Profile +y follows `up` (default +Z); corners are
 * mitered so walls keep their thickness. Always capped, never closed.
 */
export function sweep(profile, path, opts) {
  const o = checkOpts(opts, ['segments', 'up', 'curveSegments'], 'sweep');
  const polys = toPolygons(profile, o.curveSegments);
  let reach = 0;
  for (const poly of polys) {
    for (const [x, y] of poly) reach = Math.max(reach, Math.hypot(x, y));
  }
  const frames = sweepFrames(pathStations(path, o), o, reach);
  return new Solid(ops().op_solid_sweep(polys, frames));
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
  // f64 across the boundary: lossless for any input (Float32Array included).
  const positions = posAttr.array instanceof Float64Array ? posAttr.array : Float64Array.from(posAttr.array);
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
// walks the declared schema and returns real THREE instances at every
// extension-typed position (any depth), fills absent object properties from
// their declared defaults, and selects union branches by tag. Colors stay
// in their wire form (hex string or [r, g, b]) — exactly what .color()
// takes. Plain args arrive already normalized and default-filled by the
// engine; cascade values arrive as provided, so the fill here is what makes
// both channels read the same.
function hydrate(schema, v) {
  if (!schema || typeof schema !== 'object') return reviveValue(v);
  if (schema.variants && v && typeof v === 'object' && !Array.isArray(v)) {
    const tag = schema.tag ?? 'kind';
    const body = schema.variants[v[tag]];
    if (body) return hydrateObject(body.properties, v);
  }
  const type = schema.type;
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
  }
  if (type === 'array' && schema.items && Array.isArray(v)) {
    return v.map((el) => hydrate(schema.items, el));
  }
  if (type === 'object' && v && typeof v === 'object' && !Array.isArray(v)) {
    if (schema.properties) return hydrateObject(schema.properties, v);
    if (schema.additionalProperties) {
      const out = {};
      for (const [k, val] of Object.entries(v)) out[k] = hydrate(schema.additionalProperties, val);
      return out;
    }
  }
  // 'solid' arrives as a tagged handle that reviveValue turns back into
  // a Solid; plain JSON values may carry nested Solids too.
  return reviveValue(v);
}

// An object value against a properties map: hydrate declared keys, revive
// the rest (a union's tag, pass-through extras), fill absent declared
// defaults.
function hydrateObject(props, v) {
  const out = {};
  for (const [k, val] of Object.entries(v)) {
    const p = props?.[k];
    out[k] = p ? hydrate(p, val) : reviveValue(val);
  }
  for (const [k, p] of Object.entries(props ?? {})) {
    if (!(k in out) && p.default !== undefined) out[k] = hydrate(p, p.default);
  }
  return out;
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
      return hydrate(decl.schema, raw);
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
    sweep,
    fromThreeGeometry,
    group,
    deg,
  };

  const log = (level) => (...a) => {
    const o = globalThis.__odmOps ?? globalThis.Deno?.core?.ops;
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
