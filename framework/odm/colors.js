// Named colors (CSS values). Kept to a well-known subset; hex works for
// everything else.
const NAMED = {
  black: 0x000000, white: 0xffffff, gray: 0x808080, grey: 0x808080,
  silver: 0xc0c0c0, lightgray: 0xd3d3d3, lightgrey: 0xd3d3d3,
  darkgray: 0xa9a9a9, darkgrey: 0xa9a9a9, dimgray: 0x696969, dimgrey: 0x696969,
  red: 0xff0000, darkred: 0x8b0000, crimson: 0xdc143c, firebrick: 0xb22222,
  salmon: 0xfa8072, coral: 0xff7f50, tomato: 0xff6347, orangered: 0xff4500,
  orange: 0xffa500, darkorange: 0xff8c00, gold: 0xffd700,
  yellow: 0xffff00, khaki: 0xf0e68c, ivory: 0xfffff0, beige: 0xf5f5dc,
  brown: 0xa52a2a, saddlebrown: 0x8b4513, sienna: 0xa0522d, chocolate: 0xd2691e,
  peru: 0xcd853f, tan: 0xd2b48c, wheat: 0xf5deb3,
  green: 0x008000, darkgreen: 0x006400, lime: 0x00ff00, limegreen: 0x32cd32,
  forestgreen: 0x228b22, seagreen: 0x2e8b57, olive: 0x808000,
  springgreen: 0x00ff7f, yellowgreen: 0x9acd32, lightgreen: 0x90ee90,
  teal: 0x008080, cyan: 0x00ffff, aqua: 0x00ffff, turquoise: 0x40e0d0,
  blue: 0x0000ff, navy: 0x000080, darkblue: 0x00008b, mediumblue: 0x0000cd,
  royalblue: 0x4169e1, steelblue: 0x4682b4, dodgerblue: 0x1e90ff,
  deepskyblue: 0x00bfff, skyblue: 0x87ceeb, lightblue: 0xadd8e6,
  cornflowerblue: 0x6495ed, slateblue: 0x6a5acd, cadetblue: 0x5f9ea0,
  purple: 0x800080, indigo: 0x4b0082, violet: 0xee82ee, magenta: 0xff00ff,
  fuchsia: 0xff00ff, orchid: 0xda70d6, plum: 0xdda0dd, lavender: 0xe6e6fa,
  hotpink: 0xff69b4, pink: 0xffc0cb, deeppink: 0xff1493,
  maroon: 0x800000, slategray: 0x708090, slategrey: 0x708090,
  lightslategray: 0x778899, lightslategrey: 0x778899,
  midnightblue: 0x191970, goldenrod: 0xdaa520, darkgoldenrod: 0xb8860b,
  rebeccapurple: 0x663399,
};

function srgbToLinear(c) {
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

function fromInt(n, a = 1) {
  return [
    srgbToLinear(((n >> 16) & 0xff) / 255),
    srgbToLinear(((n >> 8) & 0xff) / 255),
    srgbToLinear((n & 0xff) / 255),
    a,
  ];
}

/**
 * Parse a color into linear RGBA. Accepts named CSS colors, '#rgb'/'#rrggbb'
 * hex strings, 0xRRGGBB numbers, and [r,g,b] / [r,g,b,a] arrays of sRGB
 * values in 0..1.
 */
export function parseColor(c) {
  if (Array.isArray(c)) {
    if (c.length < 3 || c.length > 4 || c.some((v) => typeof v !== 'number')) {
      throw new TypeError(`color array must be [r,g,b] or [r,g,b,a] in 0..1, got ${JSON.stringify(c)}`);
    }
    if (c.length === 4 && c[3] !== 1) {
      // The renderer has no blending yet; a silently opaque 0.3 would mislead.
      throw new TypeError(`translucent colors are not supported yet: alpha must be 1, got ${c[3]}`);
    }
    return [srgbToLinear(c[0]), srgbToLinear(c[1]), srgbToLinear(c[2]), c.length === 4 ? c[3] : 1];
  }
  if (typeof c === 'number') return fromInt(c);
  if (typeof c === 'string') {
    const s = c.trim().toLowerCase();
    if (s in NAMED) return fromInt(NAMED[s]);
    let m = /^#([0-9a-f]{6})$/.exec(s);
    if (m) return fromInt(parseInt(m[1], 16));
    m = /^#([0-9a-f]{3})$/.exec(s);
    if (m) {
      const [r, g, b] = m[1];
      return fromInt(parseInt(r + r + g + g + b + b, 16));
    }
    throw new TypeError(
      `unknown color '${c}': use a hex string like '#4682b4', an [r,g,b] array, or a common CSS name`,
    );
  }
  throw new TypeError(`cannot parse color from ${typeof c}`);
}
