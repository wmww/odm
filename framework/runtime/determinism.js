// Executed at snapshot time; the frozen state is baked into every isolate.
// Date is fixed and Math.random is a seeded PRNG so build() is deterministic.
(() => {
  const FIXED = 1700000000000;
  const RealDate = Date;
  class FrozenDate extends RealDate {
    constructor(...args) {
      if (args.length === 0) {
        super(FIXED);
      } else {
        super(...args);
      }
    }
    static now() {
      return FIXED;
    }
  }
  FrozenDate.parse = RealDate.parse;
  FrozenDate.UTC = RealDate.UTC;
  globalThis.Date = FrozenDate;

  let s = 0x12345678 >>> 0;
  Math.random = function seededRandom() {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
})();
