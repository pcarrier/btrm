export interface CellMetrics {
  /** CSS pixel width. */
  w: number;
  /** CSS pixel height. */
  h: number;
  /** Device pixel width (integer). */
  pw: number;
  /** Device pixel height (integer). */
  ph: number;
}

/**
 * Measure the dimensions of a single monospace cell by rendering 'M'
 * into a hidden span, then snapping to device pixel boundaries.
 */
export const CSS_GENERIC = new Set([
  "serif", "sans-serif", "monospace", "cursive", "fantasy",
  "system-ui", "ui-serif", "ui-sans-serif", "ui-monospace", "ui-rounded",
  "math", "emoji", "fangsong",
]);

/** Quote font families for CSS font shorthand (canvas ctx.font). */
export function cssFontFamily(family: string): string {
  return family
    .split(",")
    .map((f) => {
      f = f.trim();
      if (CSS_GENERIC.has(f.toLowerCase())) return f;
      if (f.startsWith('"') || f.startsWith("'")) return f;
      return `'${f}'`;
    })
    .join(", ");
}

export function measureCell(fontFamily: string, fontSize: number, dpr?: number): CellMetrics {
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d")!;
  ctx.font = `${fontSize}px ${cssFontFamily(fontFamily)}`;
  const metrics = ctx.measureText("M");
  const w = metrics.width;
  // Use font metrics for accurate height: ascent + descent gives the full
  // glyph extent. This matches how real terminals compute cell height from
  // the font's ascender/descender values.
  const h = metrics.fontBoundingBoxAscent + metrics.fontBoundingBoxDescent;

  const d = dpr ?? (window.devicePixelRatio || 1);
  const pw = Math.round(w * d);
  const ph = Math.round(h * d);
  return { w: pw / d, h: ph / d, pw, ph };
}
