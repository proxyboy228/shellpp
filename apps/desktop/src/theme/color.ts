// Color parsing, alpha compositing and the WCAG 2.x contrast ratio.
// Used by the contrast test and by code that picks colors for a terminal
// background (docs/design-system.md, 4.3 and 7.3).

/** Red, green and blue in 0..255 (may be fractional after compositing), alpha in 0..1. */
export type Rgba = readonly [r: number, g: number, b: number, a: number];

const HEX_RE = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})?$/i;
const NUMBER = String.raw`\s*(\d+(?:\.\d+)?|\.\d+)\s*`;
const RGB_RE = new RegExp(String.raw`^rgba?\(${NUMBER},${NUMBER},${NUMBER}(?:,${NUMBER})?\)$`, 'i');

/**
 * Parses `#RRGGBB`, `#RRGGBBAA`, `rgb(r, g, b)` and `rgba(r, g, b, a)`.
 * Throws on anything else, so that an unexpected token format fails loudly.
 */
export function parseColor(value: string): Rgba {
  const text = value.trim();

  const hex = HEX_RE.exec(text);
  if (hex !== null) {
    const [, r, g, b, a] = hex;
    return [
      Number.parseInt(r, 16),
      Number.parseInt(g, 16),
      Number.parseInt(b, 16),
      a === undefined ? 1 : Number.parseInt(a, 16) / 255,
    ];
  }

  const rgb = RGB_RE.exec(text);
  if (rgb !== null) {
    const [, r, g, b, a] = rgb;
    const color: Rgba = [Number(r), Number(g), Number(b), a === undefined ? 1 : Number(a)];
    if (color.slice(0, 3).some((channel) => channel > 255) || color[3] > 1) {
      throw new Error(`Color channel out of range: ${value}`);
    }
    return color;
  }

  throw new Error(`Unsupported color value: ${value}`);
}

/** Returns true when the color has no transparency. */
export function isOpaque(color: Rgba): boolean {
  return color[3] === 1;
}

/** Draws `top` over an opaque `bottom` (source-over). The result is opaque and not rounded. */
export function composite(top: Rgba, bottom: Rgba): Rgba {
  if (!isOpaque(bottom)) {
    throw new Error('The bottom color of a composite must be opaque');
  }
  const alpha = top[3];
  return [
    top[0] * alpha + bottom[0] * (1 - alpha),
    top[1] * alpha + bottom[1] * (1 - alpha),
    top[2] * alpha + bottom[2] * (1 - alpha),
    1,
  ];
}

function linearize(channel: number): number {
  const value = channel / 255;
  // sRGB transfer function. WCAG 2.x cites 0.03928 as the threshold; no
  // 8-bit channel value lies between 0.03928 and 0.04045, so both agree.
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

/** WCAG 2.x relative luminance of an opaque color. */
export function relativeLuminance(color: Rgba): number {
  if (!isOpaque(color)) {
    throw new Error('Relative luminance is defined only for opaque colors; composite first');
  }
  return 0.2126 * linearize(color[0]) + 0.7152 * linearize(color[1]) + 0.0722 * linearize(color[2]);
}

/** WCAG 2.x contrast ratio of two opaque colors, from 1 to 21, not rounded. */
export function contrastRatio(a: Rgba, b: Rgba): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

const BLACK: Rgba = [0, 0, 0, 1];
const WHITE: Rgba = [255, 255, 255, 1];

/**
 * A background is light when black text has a higher contrast on it than
 * white text (relative luminance above about 0.1791).
 */
export function isLightBackground(background: Rgba): boolean {
  return contrastRatio(BLACK, background) > contrastRatio(WHITE, background);
}

/** Formats a color in a canonical form, used to compare values written in different notations. */
export function formatRgba(color: Rgba): string {
  const round = (value: number) => Number(value.toFixed(4));
  return `rgba(${round(color[0])}, ${round(color[1])}, ${round(color[2])}, ${round(color[3])})`;
}
