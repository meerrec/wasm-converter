import { decodePng, encodePng } from './png.mjs';

export const RGB_TOLERANCE = 2;
function masked(mask, x, y) {
  return mask.some((r) => x >= r.x && y >= r.y && x < r.x + r.width && y < r.y + r.height);
}
function validateMasks(masks, width, height) {
  for (const mask of masks) {
    if (![mask.x, mask.y, mask.width, mask.height].every(Number.isInteger) ||
      mask.x < 0 || mask.y < 0 || mask.width <= 0 || mask.height <= 0 ||
      mask.x + mask.width > width || mask.y + mask.height > height) {
      throw new Error(`Mask is outside image bounds: ${JSON.stringify(mask)}`);
    }
  }
}
function ssim(a, b, masks) {
  // Diagnostic only: mean of per-8x8-window luminance SSIM. It never admits a match.
  const { width: w, height: h } = a;
  const values = [];
  for (let y0 = 0; y0 < h; y0 += 8) for (let x0 = 0; x0 < w; x0 += 8) {
    const left = [], right = [];
    for (let y = y0; y < Math.min(y0 + 8, h); y += 1) for (let x = x0; x < Math.min(x0 + 8, w); x += 1) {
      if (masked(masks, x, y)) continue;
      const i = (y * w + x) * 4;
      left.push(0.2126 * a.pixels[i] + 0.7152 * a.pixels[i + 1] + 0.0722 * a.pixels[i + 2]);
      right.push(0.2126 * b.pixels[i] + 0.7152 * b.pixels[i + 1] + 0.0722 * b.pixels[i + 2]);
    }
    if (left.length < 2) continue;
    const meanA = left.reduce((s, v) => s + v, 0) / left.length;
    const meanB = right.reduce((s, v) => s + v, 0) / right.length;
    let varA = 0, varB = 0, covariance = 0;
    for (let i = 0; i < left.length; i += 1) {
      varA += (left[i] - meanA) ** 2; varB += (right[i] - meanB) ** 2;
      covariance += (left[i] - meanA) * (right[i] - meanB);
    }
    const divisor = left.length - 1;
    varA /= divisor; varB /= divisor; covariance /= divisor;
    const c1 = (0.01 * 255) ** 2, c2 = (0.03 * 255) ** 2;
    values.push(((2 * meanA * meanB + c1) * (2 * covariance + c2)) /
      ((meanA ** 2 + meanB ** 2 + c1) * (varA + varB + c2)));
  }
  return values.length ? values.reduce((s, v) => s + v, 0) / values.length : null;
}

export function comparePng(referenceBytes, actualBytes, options = {}) {
  const reference = decodePng(referenceBytes), actual = decodePng(actualBytes);
  if (reference.width !== actual.width || reference.height !== actual.height) {
    return { status: 'visual-diff', reason: 'dimension-mismatch', reference: { width: reference.width, height: reference.height }, actual: { width: actual.width, height: actual.height }, diffPng: null };
  }
  const masks = options.masks ?? [];
  validateMasks(masks, reference.width, reference.height);
  const diff = Buffer.from(actual.pixels);
  let mismatchingPixels = 0, comparedPixels = 0, maxChannelDelta = 0, alphaMismatch = false;
  const bounds = { x: reference.width, y: reference.height, width: 0, height: 0 };
  let maxX = -1, maxY = -1;
  for (let y = 0; y < reference.height; y += 1) for (let x = 0; x < reference.width; x += 1) {
    if (masked(masks, x, y)) {
      const j = (y * reference.width + x) * 4;
      diff[j] = 255; diff[j + 1] = 0; diff[j + 2] = 255; diff[j + 3] = 255;
      continue;
    }
    comparedPixels += 1;
    const i = (y * reference.width + x) * 4;
    const deltas = [0, 1, 2].map((c) => Math.abs(reference.pixels[i + c] - actual.pixels[i + c]));
    const alphaDelta = Math.abs(reference.pixels[i + 3] - actual.pixels[i + 3]);
    maxChannelDelta = Math.max(maxChannelDelta, ...deltas, alphaDelta);
    const different = deltas.some((d) => d > RGB_TOLERANCE) || alphaDelta !== 0;
    alphaMismatch ||= alphaDelta !== 0;
    if (different) {
      mismatchingPixels += 1;
      bounds.x = Math.min(bounds.x, x); bounds.y = Math.min(bounds.y, y); maxX = Math.max(maxX, x); maxY = Math.max(maxY, y);
      diff[i] = 255; diff[i + 1] = 0; diff[i + 2] = 0; diff[i + 3] = 255;
    } else {
      const gray = Math.round(0.2126 * actual.pixels[i] + 0.7152 * actual.pixels[i + 1] + 0.0722 * actual.pixels[i + 2]);
      diff[i] = gray; diff[i + 1] = gray; diff[i + 2] = gray;
    }
  }
  const ssimValue = ssim(reference, actual, masks);
  return {
    status: mismatchingPixels === 0 ? 'match' : 'visual-diff', reason: mismatchingPixels === 0 ? null : 'pixels',
    tolerance: { rgb: RGB_TOLERANCE, alpha: 'exact' }, comparedPixels, mismatchingPixels,
    mismatchRatio: comparedPixels ? mismatchingPixels / comparedPixels : 0,
    maxChannelDelta, alphaMismatch, ssim: ssimValue, masks,
    diffBounds: maxX < 0 ? null : { ...bounds, width: maxX - bounds.x + 1, height: maxY - bounds.y + 1 },
    diffPng: encodePng({ width: reference.width, height: reference.height, pixels: diff }),
  };
}
