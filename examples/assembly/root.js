//! ODM API unstable
//! Part composition: one wheel geometry, four placements — content
//! addressing stores the wheel mesh once.
export default function build(ctx) {
  const wheel = ctx.invoke('parts/wheel.js', { radius: 8 });
  const chassis = odm.box([70, 30, 10]).translate(0, 0, 16).color('#dc143c').name('chassis');

  // Wheels are built along Z; lay them on their side so the axle runs along Y.
  const laid = wheel.rotateX(odm.deg(90));
  return odm.group(
    chassis,
    laid.translate(-24, -18, 8).name('wheel-fl'),
    laid.translate(24, -18, 8).name('wheel-fr'),
    laid.translate(-24, 18, 8).name('wheel-rl'),
    laid.translate(24, 18, 8).name('wheel-rr'),
  ).name('cart');
}
