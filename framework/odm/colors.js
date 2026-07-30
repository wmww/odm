function srgbToLinear(c) {
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

function fromInt(n) {
  return [
    srgbToLinear(((n >> 16) & 0xff) / 255),
    srgbToLinear(((n >> 8) & 0xff) / 255),
    srgbToLinear((n & 0xff) / 255),
    1,
  ];
}

/**
 * Parse a color into linear RGBA. Accepts '#rrggbb'/'#rgb' hex strings and
 * [r,g,b] / [r,g,b,a] arrays of sRGB values in 0..1.
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
  if (typeof c === 'string') {
    const s = c.trim().toLowerCase();
    let m = /^#([0-9a-f]{6})$/.exec(s);
    if (m) return fromInt(parseInt(m[1], 16));
    m = /^#([0-9a-f]{3})$/.exec(s);
    if (m) {
      const [r, g, b] = m[1];
      return fromInt(parseInt(r + r + g + g + b + b, 16));
    }
    throw new TypeError(
      `invalid color '${c}': use a hex string like '#4682b4' or an [r,g,b] array of 0..1 sRGB values (named colors are not supported)`,
    );
  }
  if (typeof c === 'number' && Number.isInteger(c) && c >= 0 && c <= 0xffffff) {
    throw new TypeError(
      `number colors are not supported: write '#${c.toString(16).padStart(6, '0')}' instead`,
    );
  }
  throw new TypeError(`cannot parse color from ${typeof c}: use a hex string like '#4682b4' or an [r,g,b] array`);
}
