//! odm unstable
// One invoke level between root.js and block.js: instances nest, and the
// cascade passes through a file that never mentions the input.
export default function build(ctx) {
  return ctx.invoke('block.js').translate(0, 0, 5);
}
