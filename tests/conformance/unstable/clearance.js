//! odm unstable
// s.clearance(other): exact overlap, AABB-derived gap lower bound. The
// assertions run inside build() — a wrong answer throws, which the runner
// reports as a failed build. All values are exact box arithmetic.
export default function build() {
  const a = odm.box(2); // spans [-1, 1]³
  const expect = (c, overlap, gap) => {
    if (c.overlap !== overlap || Math.abs(c.gap_lower_bound - gap) > 1e-9) {
      throw new Error(`clearance ${JSON.stringify(c)}, wanted ${overlap}/${gap}`);
    }
  };

  // Separated by 3 between faces; the axis-aligned gap is exact.
  expect(a.clearance(a.translate(5, 0, 0)), false, 3);
  // Diagonal separation: componentwise box distance, √(3² + 4²).
  expect(a.clearance(a.translate(5, 6, 0)), false, 5);
  // Interpenetration is overlap; exact face contact is not.
  expect(a.clearance(a.translate(1, 0, 0)), true, 0);
  expect(a.clearance(a.translate(2, 0, 0)), false, 0);
  // Overlapping boxes, disjoint solids (an L around a corner): overlap is
  // decided on real geometry, and the gap bound honestly stays 0.
  const ell = odm.box(2).subtract(odm.box(2).translate(1, 1, 0));
  expect(ell.clearance(a.translate(1.5, 1.5, 0)), false, 0);
  // The pending transform participates (baked, like every query).
  expect(a.translate(10, 0, 0).clearance(a.translate(15, 0, 0)), false, 3);

  return a.name('probe');
}

export const checks = [{ volume: [8, 1e-9] }];
