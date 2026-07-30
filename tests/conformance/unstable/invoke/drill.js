//! odm unstable
// Helper for invoke/main.js: bores a hole through whatever Solid it is given.
export default function build(ctx) {
  const { blank, r } = ctx.args;
  console.log(`drilling r=${r}`);
  return blank.subtract(odm.cylinder({ r, h: 100 }));
}
