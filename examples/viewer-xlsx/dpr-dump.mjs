// Временный анализ устойчивости метрики сдвига: чернильный и градиентный профили.
import { chromium } from '@playwright/test';

const URL = 'http://localhost:5179/';
const VIEWPORT = { width: 1100, height: 620 };

const browser = await chromium.launch();

async function open(context) {
  const page = await context.newPage();
  await page.goto(URL);
  await page.waitForFunction(() => window.docConverter !== undefined);
  await page.selectOption('#fixture', 'content-dense.xlsx');
  await page.waitForFunction(() => document.querySelector('#cmds')?.textContent !== '0', null, { timeout: 30000 });
  await page.waitForTimeout(300);
  return page;
}

async function pngBase64(page) {
  return page.evaluate(async () => {
    const bytes = await window.docConverter.viewer.exportPng();
    let s = '';
    for (let i = 0; i < bytes.length; i += 0x4000) {
      const chunk = bytes.subarray(i, i + 0x4000);
      for (let j = 0; j < chunk.length; j += 1) s += String.fromCharCode(chunk[j]);
    }
    return btoa(s);
  });
}

const ctx1 = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1 });
const ctx2 = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 2 });

for (let round = 1; round <= 2; round += 1) {
  const p1 = await open(ctx1);
  const b64ref = await pngBase64(p1);
  const p2 = await open(ctx2);
  const b64own = await pngBase64(p2);
  const out = await p2.evaluate(async (arg) => {
    const decode = async (b64) => {
      const bin = atob(b64);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i);
      const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext('2d');
      ctx.drawImage(bitmap, 0, 0);
      const img = ctx.getImageData(0, 0, bitmap.width, bitmap.height);
      return { w: bitmap.width, h: bitmap.height, data: img.data };
    };
    const own = await decode(arg.own);
    const ref = await decode(arg.ref);
    const l = (d, w, x, y) => { const i = (y * w + x) * 4; return 0.299 * d[i] + 0.587 * d[i + 1] + 0.114 * d[i + 2]; };
    const profX = (img, f) => { const a = new Float64Array(img.w); for (let x = 0; x < img.w; x += 1) { let s = 0; for (let y = 0; y < img.h; y += 1) s += f(img, x, y); a[x] = s; } return a; };
    const profY = (img, f) => { const a = new Float64Array(img.h); for (let y = 0; y < img.h; y += 1) { let s = 0; for (let x = 0; x < img.w; x += 1) s += f(img, x, y); a[y] = s; } return a; };
    const ink = (img, x, y) => 255 - l(img.data, img.w, x, y);
    const gradX = (img, x, y) => (x + 1 < img.w ? Math.abs(l(img.data, img.w, x + 1, y) - l(img.data, img.w, x, y)) : 0);
    const gradY = (img, x, y) => (y + 1 < img.h ? Math.abs(l(img.data, img.w, x, y + 1) - l(img.data, img.w, x, y)) : 0);
    const pair = (a) => { const r = new Float64Array(Math.floor(a.length / 2)); for (let i = 0; i < r.length; i += 1) r[i] = a[2 * i] + a[2 * i + 1]; return r; };
    const sad = (a, b, d) => { let s = 0; const from = Math.max(0, -d), to = Math.min(a.length, b.length - d); for (let i = from; i < to; i += 1) s += Math.abs(a[i] - b[i + d]); return s / (to - from); };
    const best = (a, b) => { const vals = []; let bd = 0, bv = Infinity; for (let d = -4; d <= 4; d += 1) { const v = sad(a, b, d); vals.push(+v.toFixed(0)); if (v < bv) { bv = v; bd = d; } } return { best: bd, vals }; };
    return {
      inkX: best(profX(ref, ink), pair(profX(own, ink))),
      inkY: best(profY(ref, ink), pair(profY(own, ink))),
      gradX: best(profX(ref, gradX), pair(profX(own, gradX))),
      gradY: best(profY(ref, gradY), pair(profY(own, gradY))),
    };
  }, { ref: b64ref, own: b64own });
  console.log('round', round, JSON.stringify(out));
}

await browser.close();
