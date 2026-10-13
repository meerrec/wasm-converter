/**
 * Тесты сохранения на диск: гард вне браузера, путь File System Access API (успех,
 * отмена, опции без description) и фолбэк через `<a download>` + object URL.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';

import { saveFileToDisk } from '../src/save.js';

const DOCX_MIME = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('saveFileToDisk', () => {
  it('вне браузера бросает понятную ошибку', async () => {
    vi.unstubAllGlobals();
    await expect(saveFileToDisk(new Uint8Array([1]), 'a.bin')).rejects.toThrow(
      'only in a browser',
    );
  });

  it('сохраняет через File System Access API', async () => {
    const write = vi.fn().mockResolvedValue(undefined);
    const close = vi.fn().mockResolvedValue(undefined);
    const createWritable = vi.fn().mockResolvedValue({ write, close });
    const showSaveFilePicker = vi.fn().mockResolvedValue({ createWritable });
    vi.stubGlobal('window', { showSaveFilePicker });
    vi.stubGlobal('document', {});

    const ok = await saveFileToDisk(new Uint8Array([1, 2, 3]), 'report.docx', {
      description: 'Word document',
    });

    expect(ok).toBe(true);
    expect(showSaveFilePicker).toHaveBeenCalledWith({
      suggestedName: 'report.docx',
      types: [{ description: 'Word document', accept: { [DOCX_MIME]: ['.docx'] } }],
    });
    expect(createWritable).toHaveBeenCalledTimes(1);
    expect(write).toHaveBeenCalledTimes(1);
    expect(close).toHaveBeenCalledTimes(1);

    // Uint8Array обёрнут в Blob с MIME-типом по расширению.
    const blob = write.mock.calls[0]![0]! as Blob;
    expect(blob.type).toBe(DOCX_MIME);
  });

  it('без description не передаёт types в диалог', async () => {
    const showSaveFilePicker = vi.fn().mockResolvedValue({
      createWritable: vi.fn().mockResolvedValue({
        write: vi.fn().mockResolvedValue(undefined),
        close: vi.fn().mockResolvedValue(undefined),
      }),
    });
    vi.stubGlobal('window', { showSaveFilePicker });
    vi.stubGlobal('document', {});

    const ok = await saveFileToDisk(new Uint8Array([1]), 'data.csv');

    expect(ok).toBe(true);
    expect(showSaveFilePicker).toHaveBeenCalledWith({ suggestedName: 'data.csv' });
  });

  it('отмена диалога → false без ошибки', async () => {
    const showSaveFilePicker = vi
      .fn()
      .mockRejectedValue(new DOMException('canceled by user', 'AbortError'));
    vi.stubGlobal('window', { showSaveFilePicker });
    vi.stubGlobal('document', {});

    await expect(saveFileToDisk(new Uint8Array([1]), 'report.docx')).resolves.toBe(false);
  });

  it('прочие ошибки диалога пробрасываются', async () => {
    const showSaveFilePicker = vi
      .fn()
      .mockRejectedValue(new DOMException('permission denied', 'NotAllowedError'));
    vi.stubGlobal('window', { showSaveFilePicker });
    vi.stubGlobal('document', {});

    await expect(saveFileToDisk(new Uint8Array([1]), 'report.docx')).rejects.toThrow(
      'permission denied',
    );
  });

  it('фолбэк без picker: скачивание через <a download> + object URL', async () => {
    const click = vi.fn();
    const anchor = { href: '', download: '', click } as unknown as HTMLAnchorElement;
    const createElement = vi.fn().mockReturnValue(anchor);
    vi.stubGlobal('window', {});
    vi.stubGlobal('document', { createElement });
    const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:test');
    const revokeObjectURL = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});

    const ok = await saveFileToDisk(new Blob(['x'], { type: 'text/plain' }), 'out.txt');

    expect(ok).toBe(true);
    expect(createElement).toHaveBeenCalledWith('a');
    expect(anchor.href).toBe('blob:test');
    expect(anchor.download).toBe('out.txt');
    expect(click).toHaveBeenCalledTimes(1);
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:test');

    createObjectURL.mockRestore();
    revokeObjectURL.mockRestore();
  });
});
