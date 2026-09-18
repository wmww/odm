//! ODM API unstable
// sweep(profile, path, opts): rotation-minimizing frames along a polyline,
// mitered corners, always capped. All three volumes here are exact.
export default function build() {
  // A straight path along +Z is exactly an extrude: 20x10 profile, height 4.
  const rect = [[0, 0], [20, 0], [20, 10], [0, 10]];
  const straight = odm.sweep(rect, [[0, 0, 0], [0, 0, 4]]).name('straight');

  // 90° elbow, 2x2 profile centred on the path, legs 10 and 6 to the corner.
  // The miter removes an inner wedge and adds an equal outer one, so the
  // volume is exactly a²·(L1 + L2) = 4·16.
  const a = 2;
  const sq = [[-a / 2, -a / 2], [a / 2, -a / 2], [a / 2, a / 2], [-a / 2, a / 2]];
  const elbow = odm
    .sweep(sq, [[0, 0, 0], [10, 0, 0], [10, 6, 0]])
    .name('elbow')
    .translate(50, 0, 0);

  // A ribbon: 10 wide, 0.5 thick, laid flat by up = +Z (also the default).
  const ribbon = odm
    .sweep([[-5, -0.25], [5, -0.25], [5, 0.25], [-5, 0.25]], [[0, 0, 0], [20, 0, 0]], {
      up: [0, 0, 1],
    })
    .name('ribbon')
    .translate(100, 0, 0);

  // Holes ride along: the same elbow as a square tube with a 2x2 bore,
  // both loops mitered by the same frame, so (4² - 2²)·(10 + 6).
  const ring = (w) => [[-w, -w], [w, -w], [w, w], [-w, w]];
  const tube = odm
    .sweep([ring(2), ring(1)], [[0, 0, 0], [10, 0, 0], [10, 6, 0]])
    .name('tube')
    .translate(150, 0, 0);

  return odm.group(straight, elbow, ribbon, tube);
}

export const checks = [
  { volume: [800 + 64 + 100 + 12 * 16, 1e-9] },
  { bounds: { min: [0, -5, -2], max: [162, 10, 4], eps: 1e-9 } },
  // Profile +y follows `up`, so the ribbon's flat face is its top at z = 0.25.
  { raycast: { origin: [110, 0, 50], dir: [0, 0, -1], distance: [49.75, 1e-9], normal: [0, 0, 1], name: 'ribbon' } },
  // The elbow is an L, not a filled square: leg 1 is hit, the pocket is not.
  { raycast: { origin: [55, 0, 50], dir: [0, 0, -1], distance: [49, 1e-9], normal: [0, 0, 1], name: 'elbow' } },
  { raycast: { origin: [55, 3, 50], dir: [0, 0, -1], miss: true } },
];
