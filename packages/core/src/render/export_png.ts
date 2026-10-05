/**
 * Экспорт OffscreenCanvas в PNG.
 * Работает из воркера с Chrome 108 / Firefox 116 / Safari 16.4.
 */
export async function exportPng(canvas: OffscreenCanvas): Promise<Uint8Array> {
  const blob = await canvas.convertToBlob({ type: 'image/png' });
  const ab = await blob.arrayBuffer();
  return new Uint8Array(ab);
}
