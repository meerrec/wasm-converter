/**
 * Экспорт холста в PNG.
 *
 * У `OffscreenCanvas` (воркер-путь) есть `convertToBlob`, у обычного
 * `HTMLCanvasElement` (main-режим) — только колбэк `toBlob`. Обе ветки отдают
 * одинаковые байты, поэтому хост разницы не видит.
 */
export async function exportPng(
  canvas: OffscreenCanvas | HTMLCanvasElement,
): Promise<Uint8Array> {
  if ('convertToBlob' in canvas) {
    const blob = await canvas.convertToBlob({ type: 'image/png' });
    const ab = await blob.arrayBuffer();
    return new Uint8Array(ab);
  }
  return new Promise<Uint8Array>((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (!blob) {
        reject(new Error('toBlob returned null'));
        return;
      }
      blob.arrayBuffer().then(
        (ab) => resolve(new Uint8Array(ab)),
        (e: unknown) => reject(e instanceof Error ? e : new Error(String(e))),
      );
    }, 'image/png');
  });
}
