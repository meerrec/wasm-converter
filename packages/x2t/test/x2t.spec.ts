/**
 * Тесты чистых частей обвязки: сборка `params.xml`, нормализация расширений, имя файла в
 * MEMFS, формат ошибки. Настоящий wasm здесь не поднимается: модуль подменён заглушкой с той
 * поверхностью, которой пользуется обвязка (`FS` и `ccall`), а glue-скрипт — строкой, которую
 * отдаёт подменённый `fetch`. Поэтому пакет зеленеет и без `x2t.wasm`.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';

import { X2tError, convert, loadX2t } from '../src/index.js';
import type { X2tFS, X2tModule } from '../src/index.js';

/** Пути, которые заглушка `main1` вычитала из `params.xml`. */
interface StubPaths {
  readonly from: string;
  readonly to: string;
}

interface Stub {
  readonly module: X2tModule;
  /** Файлы MEMFS: путь → содержимое. Каталоги — отдельным множеством, как в `analyzePath`. */
  readonly files: Map<string, Uint8Array | string>;
  /** Путь `params.xml`, с которым звали `main1`, — по одному на вызов. */
  readonly calls: string[];
  /** Содержимое `params.xml` каждого вызова: сам файл `convert` убирает из MEMFS. */
  readonly params: string[];
}

interface StubOptions {
  /** Что делает `main1`: по умолчанию кладёт в выход `OUT` и возвращает 0. */
  readonly onMain?: (paths: StubPaths, files: Map<string, Uint8Array | string>) => number;
  /** Заставить `ccall` бросить — так проверяется формат ошибки рантайма. */
  readonly throwOnMain?: Error;
}

/** Читает значение тега `params.xml`; экранирование не разворачивает — тесты ждут его как есть. */
function tag(xml: string, name: string): string {
  const match = new RegExp(`<${name}>([^<]*)</${name}>`).exec(xml);
  if (match?.[1] === undefined) {
    throw new Error(`нет тега ${name} в params.xml:\n${xml}`);
  }
  return match[1];
}

function createStub(options: StubOptions = {}): Stub {
  const files = new Map<string, Uint8Array | string>();
  const dirs = new Set<string>(['/', '/working', '/working/fonts', '/working/themes']);
  const calls: string[] = [];
  const params: string[] = [];

  const fs = {
    mkdir: (path: string) => {
      dirs.add(path);
    },
    writeFile: (path: string, data: Uint8Array | string) => {
      files.set(path, data);
    },
    readFile: (path: string, readOptions?: { readonly encoding?: string }) => {
      const data = files.get(path);
      if (data === undefined) {
        throw new Error(`no such file: ${path}`);
      }
      if (readOptions?.encoding === 'utf8') {
        return typeof data === 'string' ? data : new TextDecoder().decode(data);
      }
      return typeof data === 'string' ? new TextEncoder().encode(data) : data;
    },
    readdir: (path: string) => (dirs.has(path) ? [...files.keys()] : []),
    unlink: (path: string) => {
      files.delete(path);
    },
    rmdir: (path: string) => {
      dirs.delete(path);
    },
    stat: () => ({ mode: 0, size: 0, mtime: 0, ctime: 0 }),
    analyzePath: (path: string) => ({
      exists: files.has(path) || dirs.has(path),
      path,
      object: null,
    }),
    isDir: () => false,
    isFile: () => true,
  };

  const ccall = (
    ident: 'main1',
    _returnType: 'number',
    _argTypes: readonly ['string'],
    args: readonly [string],
  ): number => {
    if (ident !== 'main1') {
      throw new Error(`unexpected ccall: ${ident}`);
    }
    if (options.throwOnMain !== undefined) {
      throw options.throwOnMain;
    }
    const xml = files.get('/working/params.xml');
    if (typeof xml !== 'string') {
      throw new Error('params.xml не записан до вызова main1');
    }
    calls.push(args[0]);
    params.push(xml);
    if (options.onMain !== undefined) {
      return options.onMain({ from: tag(xml, 'm_sFileFrom'), to: tag(xml, 'm_sFileTo') }, files);
    }
    files.set(tag(xml, 'm_sFileTo'), new TextEncoder().encode('OUT'));
    return 0;
  };

  const module = { FS: fs, ccall } as unknown as X2tModule;
  return { module, files, calls, params };
}

/** `params.xml` последнего вызова: сам файл `convert` убирает из MEMFS перед возвратом. */
function paramsXml(stub: Stub): string {
  const xml = stub.params.at(-1);
  if (xml === undefined) {
    throw new Error('params.xml не доходил до main1');
  }
  return xml;
}

function decode(bytes: Uint8Array): string {
  return new TextDecoder().decode(bytes);
}

const INPUT = new TextEncoder().encode('input');

describe('convert: params.xml', () => {
  it('собирает TaskQueueDataConvert с путями рабочих каталогов', async () => {
    const stub = createStub();
    await convert(stub.module, INPUT, { from: 'docx', to: 'odt', fileName: 'one_paragraph' });

    const xml = paramsXml(stub);
    expect(xml.startsWith('<?xml version="1.0" encoding="utf-8"?>')).toBe(true);
    expect(xml).toContain('<TaskQueueDataConvert xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"');
    expect(tag(xml, 'm_sFileFrom')).toBe('/working/one_paragraph.docx');
    expect(tag(xml, 'm_sFileTo')).toBe('/working/one_paragraph.odt');
    expect(tag(xml, 'm_sFontDir')).toBe('/working/fonts/');
    expect(tag(xml, 'm_sThemeDir')).toBe('/working/themes');
    expect(tag(xml, 'm_bIsNoBase64')).toBe('false');
    // Кодировка UTF-8 (65001 → 46) и разделитель-запятая (4): без них docx → txt на
    // кириллице приезжает в системной кодировке. Числа — из `cextracttools.h` апстрима.
    expect(tag(xml, 'm_nCsvTxtEncoding')).toBe('46');
    expect(tag(xml, 'm_nCsvDelimiter')).toBe('4');
    expect(xml.endsWith('</TaskQueueDataConvert>')).toBe(true);
  });

  it('дописывает слэш к каталогу шрифтов и не удваивает его', async () => {
    const bare = createStub();
    await convert(bare.module, INPUT, { from: 'docx', to: 'odt', fontDir: '/fonts' });
    expect(tag(paramsXml(bare), 'm_sFontDir')).toBe('/fonts/');

    const withSlash = createStub();
    await convert(withSlash.module, INPUT, { from: 'docx', to: 'odt', fontDir: '/fonts/' });
    expect(tag(paramsXml(withSlash), 'm_sFontDir')).toBe('/fonts/');
  });

  it('экранирует амперсанд в имени файла', async () => {
    const stub = createStub();
    await convert(stub.module, INPUT, { from: 'docx', to: 'txt', fileName: 'a&b' });
    expect(tag(paramsXml(stub), 'm_sFileFrom')).toBe('/working/a&amp;b.docx');
  });
});

describe('convert: расширения', () => {
  it('снимает точку, пробелы и регистр', async () => {
    for (const from of ['.DOCX', ' Docx ', 'docx']) {
      const stub = createStub();
      await convert(stub.module, INPUT, { from, to: '.ODT', fileName: 'x' });
      expect(tag(paramsXml(stub), 'm_sFileFrom')).toBe('/working/x.docx');
      expect(tag(paramsXml(stub), 'm_sFileTo')).toBe('/working/x.odt');
    }
  });

  it('отвергает расширение, которое не формат', async () => {
    const stub = createStub();
    for (const from of ['', '   ', 'doc x', 'd/ocx', 'do.cx', 'do-cx']) {
      await expect(convert(stub.module, INPUT, { from, to: 'odt' })).rejects.toThrow(X2tError);
    }
    // Ни один вызов не дошёл до конвертера.
    expect(stub.calls).toHaveLength(0);
  });
});

describe('convert: имя файла в MEMFS', () => {
  it('сохраняет кириллицу, пробелы и точки в имени', async () => {
    const stub = createStub();
    await convert(stub.module, INPUT, {
      from: 'docx',
      to: 'odt',
      fileName: 'ПМТ СУС 2.1 071125.docx',
    });
    expect(tag(paramsXml(stub), 'm_sFileFrom')).toBe('/working/ПМТ СУС 2.1 071125.docx');
    expect(tag(paramsXml(stub), 'm_sFileTo')).toBe('/working/ПМТ СУС 2.1 071125.odt');
  });

  it('отбрасывает расширение входа независимо от регистра', async () => {
    const stub = createStub();
    await convert(stub.module, INPUT, { from: 'docx', to: 'txt', fileName: 'report.DOCX' });
    expect(tag(paramsXml(stub), 'm_sFileFrom')).toBe('/working/report.docx');
  });

  it('снимает разделители пути и служебные символы', async () => {
    const stub = createStub();
    await convert(stub.module, INPUT, { from: 'docx', to: 'odt', fileName: '/tmp/a<b>?c*d.docx' });
    expect(tag(paramsXml(stub), 'm_sFileFrom')).toBe('/working/tmpabcd.docx');
  });

  it('подставляет input, если от имени ничего не осталось', async () => {
    const stub = createStub();
    await convert(stub.module, INPUT, { from: 'docx', to: 'odt', fileName: '///' });
    expect(tag(paramsXml(stub), 'm_sFileFrom')).toBe('/working/input.docx');
  });

  it('обрезает слишком длинное имя до 200 символов', async () => {
    const stub = createStub();
    await convert(stub.module, INPUT, { from: 'docx', to: 'odt', fileName: 'я'.repeat(250) });
    expect(tag(paramsXml(stub), 'm_sFileFrom')).toBe(`/working/${'я'.repeat(200)}.docx`);
  });

  it('называет выход по умолчанию input', async () => {
    const stub = createStub();
    await convert(stub.module, INPUT, { from: 'docx', to: 'odt' });
    expect(tag(paramsXml(stub), 'm_sFileTo')).toBe('/working/input.odt');
  });
});

describe('convert: результат и уборка', () => {
  it('возвращает байты выхода, код и пустой stderr', async () => {
    const stub = createStub();
    const result = await convert(stub.module, INPUT, { from: 'docx', to: 'txt' });
    expect(result.code).toBe(0);
    expect(decode(result.bytes)).toBe('OUT');
    expect(result.stderr).toBe('');
  });

  it('отдаёт ненулевой код как данные, а не как исключение', async () => {
    const stub = createStub({ onMain: () => 9 });
    const result = await convert(stub.module, INPUT, { from: 'docx', to: 'pdf' });
    expect(result.code).toBe(9);
    expect(result.bytes.byteLength).toBe(0);
  });

  it('оборачивает падение рантайма в X2tError и сохраняет код -1', async () => {
    const stub = createStub({ throwOnMain: new Error('boom') });
    const error = await convert(stub.module, INPUT, { from: 'docx', to: 'odt' }).catch(
      (caught: unknown) => caught,
    );
    expect(error).toBeInstanceOf(X2tError);
    const x2tError = error as X2tError;
    expect(x2tError.message).toBe('x2t crashed during conversion: boom');
    expect(x2tError.code).toBe(-1);
    expect(x2tError.stderr).toBe('');
  });

  it('убирает вход, params.xml и выход из MEMFS после вызова', async () => {
    const stub = createStub();
    await convert(stub.module, INPUT, { from: 'docx', to: 'odt', fileName: 'x' });
    expect([...stub.files.keys()]).toEqual([]);
  });

  it('не оставляет выход прошлого вызова в stderr следующего', async () => {
    const stub = createStub();
    await convert(stub.module, INPUT, { from: 'docx', to: 'odt' });
    const second = await convert(stub.module, INPUT, { from: 'docx', to: 'odt' });
    expect(second.stderr).toBe('');
    expect(stub.calls).toHaveLength(2);
  });
});

describe('loadX2t', () => {
  afterEach(() => {
    delete (globalThis as { Module?: unknown }).Module;
    vi.unstubAllGlobals();
  });

  /** Подменяет сеть: glue-скрипт и wasm приезжают строками самого теста. */
  function stubNetwork(options: { readonly script?: string; readonly wasmStatus?: number } = {}) {
    const requested: string[] = [];
    const fetchStub = (input: string | URL | Request): Promise<Response> => {
      const url = String(input instanceof Request ? input.url : input);
      requested.push(url);
      if (url.endsWith('.wasm') && options.wasmStatus !== undefined && options.wasmStatus !== 200) {
        return Promise.resolve(
          new Response('', { status: options.wasmStatus, statusText: 'Not Found' }),
        );
      }
      const body = url.endsWith('.wasm') ? new Uint8Array([0, 1, 2]) : (options.script ?? '');
      return Promise.resolve(new Response(body, { status: 200 }));
    };
    vi.stubGlobal('fetch', vi.fn(fetchStub));
    return requested;
  }

  /** Glue-скрипт-заглушка: забирает `FS`/`ccall` у теста и объявляет рантайм готовым. */
  const FAKE_GLUE = `
    globalThis.__x2tSpikeSeen = { wasmBinary: Module.wasmBinary !== undefined, noInitialRun: Module.noInitialRun };
    Module.FS = globalThis.__x2tSpikeStub.FS;
    Module.ccall = globalThis.__x2tSpikeStub.ccall;
    Module.printErr('ready');
    Module.onRuntimeInitialized();
  `;

  it('поднимает модуль, создаёт рабочие каталоги и отдаёт wasm через wasmBinary', async () => {
    const stub = createStub();
    (globalThis as { __x2tSpikeStub?: unknown }).__x2tSpikeStub = stub.module;
    const requested = stubNetwork({ script: FAKE_GLUE });

    const module = await loadX2t({ scriptUrl: '/x2t/x2t.js', wasmUrl: '/x2t/x2t.wasm' });

    expect(requested).toEqual(['/x2t/x2t.wasm', '/x2t/x2t.js']);
    expect(module.ccall).toBe(stub.module.ccall);
    const created = (globalThis as { __x2tSpikeSeen?: unknown }).__x2tSpikeSeen;
    expect(created).toEqual({ wasmBinary: true, noInitialRun: true });
    delete (globalThis as { __x2tSpikeSeen?: unknown }).__x2tSpikeSeen;
    delete (globalThis as { __x2tSpikeStub?: unknown }).__x2tSpikeStub;
  });

  it('на главном потоке исполняет glue инъекцией <script> с выставленным currentScript', async () => {
    const stub = createStub();
    (globalThis as { __x2tSpikeStub?: unknown }).__x2tSpikeStub = stub.module;

    // Glue-пробник web-ветки x2t.js: падает, если currentScript потерян (косвенный eval)
    // или элемент не отдаёт ожидаемый атрибут.
    const GLUE_WITH_PROBE = `
      if (document.currentScript === null || document.currentScript.getAttribute('data-probe') === null) {
        throw new Error('currentScript lost');
      }
      Module.FS = globalThis.__x2tSpikeStub.FS;
      Module.ccall = globalThis.__x2tSpikeStub.ccall;
      Module.onRuntimeInitialized();
    `;
    stubNetwork({ script: GLUE_WITH_PROBE });

    // Заглушка document эмулирует механику браузера: appendChild исполняет textContent
    // синхронно с временно выставленным currentScript на элемент, после — сбрасывает в null.
    let executed = false;
    const element = {
      textContent: '',
      getAttribute: () => 'seen',
    };
    const documentStub = {
      currentScript: null as typeof element | null,
      head: {
        appendChild: (el: typeof element) => {
          executed = true;
          documentStub.currentScript = el;
          (0, eval)(el.textContent);
          documentStub.currentScript = null;
        },
      },
      createElement: () => element,
    };
    vi.stubGlobal('document', documentStub);

    const module = await loadX2t({ scriptUrl: '/x2t/x2t.js' });

    expect(executed).toBe(true);
    expect(documentStub.currentScript).toBeNull();
    expect(module.ccall).toBe(stub.module.ccall);
    delete (globalThis as { __x2tSpikeStub?: unknown }).__x2tSpikeStub;
  });

  it('без document оставляет путь косвенного eval: var из glue попадает в globalThis', async () => {
    const stub = createStub();
    (globalThis as { __x2tSpikeStub?: unknown }).__x2tSpikeStub = stub.module;

    // `var __x2tEvalPath` в косвенном eval оседает на globalThis; `new Function` оставил бы
    // его локальным. Это ровно та механика, из-за которой воркерный путь — только eval.
    const EVAL_GLUE = `
      var __x2tEvalPath = 'via-eval';
      Module.FS = globalThis.__x2tSpikeStub.FS;
      Module.ccall = globalThis.__x2tSpikeStub.ccall;
      Module.onRuntimeInitialized();
    `;
    stubNetwork({ script: EVAL_GLUE });

    await loadX2t({ scriptUrl: '/x2t/x2t.js' });

    expect((globalThis as { __x2tEvalPath?: unknown }).__x2tEvalPath).toBe('via-eval');
    delete (globalThis as { __x2tEvalPath?: unknown }).__x2tEvalPath;
    delete (globalThis as { __x2tSpikeStub?: unknown }).__x2tSpikeStub;
  });

  it('отвергает второй экземпляр в той же области', async () => {
    (globalThis as { Module?: unknown }).Module = {};
    await expect(loadX2t({ scriptUrl: '/x2t/x2t.js' })).rejects.toThrow(/already loaded/);
  });

  it('сообщает HTTP-статус, если wasm не приехал', async () => {
    stubNetwork({ wasmStatus: 404 });
    const error = await loadX2t({ scriptUrl: '/x2t/x2t.js', wasmUrl: '/x2t/x2t.wasm' }).catch(
      (caught: unknown) => caught,
    );
    expect(error).toBeInstanceOf(X2tError);
    expect((error as X2tError).message).toBe('failed to fetch x2t.wasm: HTTP 404 Not Found');
  });
});

describe('X2tError', () => {
  it('по умолчанию несёт код -1 и пустой stderr', () => {
    const error = new X2tError('nope');
    expect(error.name).toBe('X2tError');
    expect(error.code).toBe(-1);
    expect(error.stderr).toBe('');
    expect(error).toBeInstanceOf(Error);
  });
});

describe('типы поверхности модуля', () => {
  it('X2tFS совпадает по форме с заглушкой', () => {
    const stub = createStub();
    const fs: X2tFS = stub.module.FS;
    expect(fs.analyzePath('/working').exists).toBe(true);
    expect(fs.analyzePath('/nope').exists).toBe(false);
  });
});
