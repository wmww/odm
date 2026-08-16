function fromInt(n, a = 1) {
  return [((n >> 16) & 0xff) / 255, ((n >> 8) & 0xff) / 255, (n & 0xff) / 255, a];
}

/**
 * Parse a color into RGBA floats in 0..1. Accepts '#rrggbb'/'#rrggbbaa'/'#rgb'
 * hex strings and [r,g,b] / [r,g,b,a] arrays. Values pass through unconverted:
 * sRGB is the one color space anything outside the renderer ever sees.
 */
export function parseColor(c) {
  if (Array.isArray(c)) {
    if (c.length < 3 || c.length > 4 || c.some((v) => typeof v !== 'number')) {
      throw new TypeError(`color array must be [r,g,b] or [r,g,b,a] in 0..1, got ${JSON.stringify(c)}`);
    }
    if (c.length === 4 && (c[3] < 0 || c[3] > 1)) {
      throw new TypeError(`color alpha must be in 0..1, got ${c[3]}`);
    }
    return [c[0], c[1], c[2], c.length === 4 ? c[3] : 1];
  }
  if (typeof c === 'string') {
    const s = c.trim().toLowerCase();
    let m = /^#([0-9a-f]{6})$/.exec(s);
    if (m) return fromInt(parseInt(m[1], 16));
    m = /^#([0-9a-f]{6})([0-9a-f]{2})$/.exec(s);
    if (m) return fromInt(parseInt(m[1], 16), parseInt(m[2], 16) / 255);
    m = /^#([0-9a-f]{3})$/.exec(s);
    if (m) {
      const [r, g, b] = m[1];
      return fromInt(parseInt(r + r + g + g + b + b, 16));
    }
    throw new TypeError(
      `invalid color '${c}': use a hex string like '#4682b4' (or '#4682b480' with alpha) or an [r,g,b] array of 0..1 values (named colors are not supported)`,
    );
  }
  if (typeof c === 'number' && Number.isInteger(c) && c >= 0 && c <= 0xffffff) {
    throw new TypeError(
      `number colors are not supported: write '#${c.toString(16).padStart(6, '0')}' instead`,
    );
  }
  throw new TypeError(`cannot parse color from ${typeof c}: use a hex string like '#4682b4' or an [r,g,b] array`);
}
