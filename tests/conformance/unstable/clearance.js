//! ODM API unstable
// s.clearance(other): signed distance. Positive = exact minimum gap with
// closest points; negative = overlap, with a separating translation whose
// length is -distance. The assertions run inside build() — a wrong answer
// throws, which the runner reports as a failed build.
export default function build() {
  const a = odm.box(2); // spans [-1, 1]³
  const near = (x, want, eps, what) => {
    if (Math.abs(x - want) > eps) throw new Error(`${what}: ${x}, wanted ${want}`);
  };

  // Separated by 3 between faces: exact distance, closest points on the
  // facing surfaces.
  let c = a.clearance(a.translate(5, 0, 0));
  near(c.distance, 3, 1e-9, 'axis gap');
  near(c.closest[0].x, 1, 1e-9, 'closest on a');
  near(c.closest[1].x, 4, 1e-9, 'closest on b');
  if (c.separate !== undefined) throw new Error('separate on a disjoint pair');

  // Diagonal separation: corner to corner, √(3² + 4²) — exact, not a box
  // bound.
  c = a.clearance(a.translate(5, 6, 0));
  near(c.distance, 5, 1e-9, 'diagonal gap');

  // Overlap by 0.5 along x: distance is -0.5 and applying `separate` to
  // the second solid clears the first (self-verifying).
  c = a.clearance(a.translate(1.5, 0, 0));
  near(c.distance, -0.5, 1e-6, 'penetration');
  near(c.separate.length(), 0.5, 1e-6, 'separation length');
  if (c.closest !== undefined) throw new Error('closest on an overlapping pair');
  const cleared = a.clearance(a.translate(1.5, 0, 0).translate(c.separate.x, c.separate.y, c.separate.z));
  if (cleared.distance < -1e-9) throw new Error(`separate did not separate: ${cleared.distance}`);

  // Exact face contact: distance ~0, and the *sign* is float noise —
  // threshold |distance| instead of nudging geometry.
  c = a.clearance(a.translate(2, 0, 0));
  near(Math.abs(c.distance), 0, 1e-9, 'tangency |distance|');

  // Overlapping boxes, disjoint solids (a cube in an L's notch): the true
  // positive wall distance, the case a box bound could never answer.
  const ell = odm.box(2).subtract(odm.box(2).translate(1, 1, 0));
  c = ell.clearance(a.translate(1.5, 1.5, 0));
  near(c.distance, 0.5, 1e-9, 'interlocked clearance');

  // The pending transform participates (baked, like every query).
  c = a.translate(10, 0, 0).clearance(a.translate(15, 0, 0));
  near(c.distance, 3, 1e-9, 'pending transforms');

  return a.name('probe');
}

export const checks = [{ volume: [8, 1e-9] }];
