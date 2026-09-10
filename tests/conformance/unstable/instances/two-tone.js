//! odm unstable
// One coloured solid and one bare one, so an Instance colour on the caller
// side has something to inherit into and something to lose to.
export default function build() {
  return odm.group(odm.box(2).name('bare'), odm.box(2).translate(4, 0, 0).color('#00ff00').name('own'));
}
