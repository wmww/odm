//! ODM API unstable
// Error contract: wrong CSG operands fail the build with the documented
// message. (Only build()-time failures can be conformance-tested this way —
// the module itself must evaluate for `checks` to be readable.)
export default function build() {
  return odm.box(1).union(odm.group(odm.box(1)));
}

export const checks = [
  { error: 'union operands must be Solids' },
  { error: 'Groups/Instances cannot be used in CSG' },
];
