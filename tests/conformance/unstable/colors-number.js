//! ODM API unstable
// 0xRRGGBB numbers are rejected — by the time the parser sees one it is
// indistinguishable from any other integer. The error spells out the fix.
export default function build() {
  return odm.box(10).color(0xff0000);
}

export const checks = [
  { error: 'number colors are not supported' },
  { error: "write '#ff0000' instead" },
];
