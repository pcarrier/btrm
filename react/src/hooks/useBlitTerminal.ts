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
export function measureCell(fontFamily: string, fontSize: number): CellMetrics {
  const span = document.createElement("span");
  span.style.cssText = `font: ${fontSize}px ${fontFamily}; position: absolute; visibility: hidden; white-space: pre;`;
  span.textContent = "M";
  document.body.appendChild(span);
  const rect = span.getBoundingClientRect();
  document.body.removeChild(span);

  const dpr = window.devicePixelRatio || 1;
  const pw = Math.round(rect.width * dpr);
  const ph = Math.round(rect.height * dpr);
  return { w: pw / dpr, h: ph / dpr, pw, ph };
}
