import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

const escape = (value) => String(value ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
export async function writeReport(outputDirectory, report) {
  await mkdir(outputDirectory, { recursive: true });
  await writeFile(path.join(outputDirectory, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  const rows = [...report.results].sort((a, b) => (b.mismatchRatio ?? 1) - (a.mismatchRatio ?? 1)).map((r) => `<article class="${escape(r.status)}"><h2>${escape(r.id)}: ${escape(r.status)}</h2><p>${escape(r.reason ?? '')}</p><p>Pixels: ${r.mismatchingPixels ?? '—'}; SSIM (diagnostic): ${r.ssim ?? '—'}</p>${r.images ? `<div class="frames"><figure><img src="${escape(r.images.reference)}"><figcaption>reference</figcaption></figure><figure><img src="${escape(r.images.actual)}"><figcaption>actual</figcaption></figure>${r.images.diff ? `<figure><img src="${escape(r.images.diff)}"><figcaption>diff / masks</figcaption></figure>` : ''}</div>` : ''}</article>`).join('\n');
  const html = `<!doctype html><html lang="ru"><meta charset="utf-8"><title>Strict viewer report</title><style>body{font:14px system-ui;max-width:1500px;margin:24px auto;padding:0 16px;background:#111827;color:#e5e7eb}a{color:#93c5fd}article{border:1px solid #374151;border-radius:8px;padding:16px;margin:16px 0}.match{border-color:#059669}.visual-diff,.page-mismatch{border-color:#dc2626}.frames{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:12px}img{width:100%;height:auto;background:white}figcaption{text-align:center}</style><h1>Strict viewer report</h1><p>Baseline: ${escape(report.baselineId ?? 'not accepted')}; revision: ${escape(report.revision ?? 'unknown')}</p><p>Exit semantics: 0 match, 1 visual/page mismatch, 2 missing reference/capture error, 3 harness error. SSIM is diagnostic only.</p>${rows}</html>`;
  await writeFile(path.join(outputDirectory, 'report.html'), html);
}
