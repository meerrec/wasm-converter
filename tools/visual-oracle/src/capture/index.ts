// Публичная поверхность захвата: одна сессия на прогон.
//
// Браузер и dev-сервер поднимаются один раз и переиспользуются всеми
// фикстурами: запуск Chromium на каждую фикстуру стоил бы дороже самого
// захвата, а сервер на каждую — ещё и пересборки wasm. Зато сбой одной
// фикстуры не роняет прогон: `capture()` не бросает исключений, а возвращает
// размеченный результат, и цикл идёт к следующей (roadmap §6.2).

import path from 'node:path';
import { chromium, type Browser } from 'playwright';
import type { RuntimeMetadata } from '../types.js';
import {
  captureFixture,
  ensureOutDir,
  writeRuntime,
  type CapturedImage,
  type CaptureFixtureContext,
  type StructureCount,
} from './capture.js';
import { asCaptureError, type CaptureErrorCode } from './errors.js';
import { startDevServer, type DevServer } from './server.js';
import type { Manifest } from '../manifest.js';
import type { DocxCaptureWindow } from './capture.js';
import type { PageWindow } from './page_api.js';

/**
 * Что нужно захвату от manifest: источник, вид, зум, локаль, таймзона и кадры.
 *
 * `Manifest` этому подходит, но захвату не нужны ни `reference`, ни `corpus`:
 * у синтетического manifest подкоманды `capture` их значения — заглушки,
 * и захват не должен их читать даже случайно.
 */
export type CapturePlan = Pick<Manifest, 'source' | 'view' | 'captures'> & {
  environment: Pick<
    Manifest['environment'],
    'applicationZoomPercent' | 'viewportCss' | 'deviceScaleFactor' | 'locale' | 'timezone'
  >;
};

/** Итог захвата одной фикстуры. Исключения через эту границу не летят. */
export type CaptureOutcome =
  | {
      ok: true;
      captures: CapturedImage[];
      runtime: RuntimeMetadata;
      /** Сколько страниц имеет документ — для структурной проверки. */
      structure: StructureCount | null;
    }
  | { ok: false; code: CaptureErrorCode; message: string };

/** Параметры сессии. */
export interface CaptureSessionOptions {
  /** Корень репозитория: отсюда запускается dev-сервер и читается git-ревизия. */
  repoRoot: string;
  /** Порт dev-сервера. По умолчанию 5178 — исторический порт просмотрщика. */
  port?: number;
  /** Каталог вывода: `<outDir>/<fixtureId>/<mode>/…`. */
  outDir: string;
  /** Сколько ждать стабильного кадра. По умолчанию 20 с. */
  stableTimeoutMs?: number;
  /** Сколько ждать открытия документа. По умолчанию 60 с. */
  openTimeoutMs?: number;
  /** Запускать Chromium с графическим окном — для отладки. */
  headless?: boolean;
}

/** Сессия захвата: браузер и сервер живут до `close()`. */
export interface CaptureSession {
  /**
   * Снять одну фикстуру в одном режиме и записать артефакты.
   *
   * @param fixture — ID фикстуры; он же имя каталога вывода.
   * @param plan — проверенный manifest этого режима либо его синтетический
   * аналог: захвату нужны только источник, вид и среда.
   */
  capture(fixture: string, plan: CapturePlan): Promise<CaptureOutcome>;
  /** Закрыть браузер и остановить сервер. Повторный вызов безопасен. */
  close(): Promise<void>;
}

/** Селектор области прокрутки вьюера: в root-режиме DOM пуст, узла нет. */
const VIEWER_SCROLLER = '#viewer > div';

/** Метка хвоста прокрутки, который окно кадра добавляет под последней страницей. */
const DOCX_TAIL_ATTR = 'data-oracle-scroll-tail';

/**
 * Окно захвата DOCX-страницы в настоящей странице.
 *
 * Границы кадра оракул вычисляет сам (`pageFrameSize`), а узлам страницы их
 * ставит этот адаптер. Область прокрутки — flex-элемент главной оси, поэтому её
 * коробке мало `height`: без `flex: none` она сожмётся обратно до видимого окна,
 * и площадь кадра снова станет окном. Поля страницы снаружи области — псевдоэлементы
 * и `background` самого `#viewer`, они размеру не мешают.
 */
export function pageDocxWindow(): DocxCaptureWindow {
  return {
    apply: async (page, size) => {
      await page.evaluate(
        ({ selector, target, tailAttr }) => {
          const doc = (globalThis as unknown as PageWindow).document;
          const node = doc.querySelector(selector);
          const style = node?.style;
          if (!node || !style) return;
          node.querySelector?.(`[${tailAttr}]`)?.remove?.();
          if (target === null) {
            style.width = '';
            style.height = '';
            style.flex = '';
            return;
          }
          style.width = `${target.width}px`;
          style.height = `${target.height}px`;
          style.flex = 'none';
          // Хвост под последней страницей: высота прокрутки — низ последней
          // страницы, и без запаса в высоту кадра браузер зажал бы
          // `scrollTop` последней страницы, кадр начался бы с предыдущей.
          // Хвост — дочерний узел распорки (`top: 100%`), поэтому следует
          // за её высотой при любой перекладке.
          const spacer = node.firstElementChild;
          if (!spacer) return;
          const tail = doc.createElement('div');
          tail.setAttribute?.(tailAttr, '');
          if (tail.style) {
            tail.style.cssText =
              'position:absolute;top:100%;left:0;width:1px;pointer-events:none;' +
              `height:${target.height}px`;
          }
          spacer.append?.(tail);
        },
        { selector: VIEWER_SCROLLER, target: size, tailAttr: DOCX_TAIL_ATTR },
      );
    },
    exportPng: async (page) => {
      const shot = await page.evaluate(async (selector) => {
        const w = globalThis as unknown as PageWindow;
        const viewer = w.docConverter?.viewer;
        const node = w.document.querySelector(selector);
        if (!viewer || !node) throw new Error('viewer bridge is gone');
        const bytes = await viewer.exportPng();
        let binary = '';
        const chunk = 0x8000;
        for (let offset = 0; offset < bytes.length; offset += chunk) {
          binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk));
        }
        // Размер читается после снимка, но до следующего кадра: если окно
        // пересчитали, это видно по `clientWidth` и подпись не соврёт.
        return {
          base64: w.btoa(binary),
          cssWidth: node.clientWidth ?? 0,
          cssHeight: node.clientHeight ?? 0,
        };
      }, VIEWER_SCROLLER);
      return {
        bytes: new Uint8Array(Buffer.from(shot.base64, 'base64')),
        cssWidth: shot.cssWidth,
        cssHeight: shot.cssHeight,
      };
    },
  };
}

const DEFAULT_PORT = 5178;

/** Каталог вывода фикстуры: `<outDir>/<fixture>/<mode>`. */
function fixtureOutDir(outDir: string, fixture: string, plan: CapturePlan): string {
  return path.join(outDir, fixture, plan.view.mode);
}

/**
 * Открыть сессию захвата.
 *
 * @throws {Error} сервер не поднялся или Chromium не запустился — это сбой
 * контура, и продолжать прогон бессмысленно.
 */
export async function createCaptureSession(
  options: CaptureSessionOptions,
): Promise<CaptureSession> {
  const port = options.port ?? DEFAULT_PORT;
  const server: DevServer = await startDevServer({ port, cwd: options.repoRoot });

  let browser: Browser;
  try {
    browser = await chromium.launch({ headless: options.headless ?? true });
  } catch (error) {
    // Браузер не поднялся — сервер уже наш, и держать его незачем.
    await server.stop();
    throw error;
  }

  let closed = false;
  const close = async (): Promise<void> => {
    if (closed) return;
    closed = true;
    // Сервер останавливается даже если браузер уже упал: `close()` браузера
    // может бросить, а порт всё равно нужно освободить.
    try {
      await browser.close();
    } catch {
      // Браузер мог умереть сам — цель всё равно достигнута.
    } finally {
      await server.stop();
    }
  };

  const capture = async (
    fixture: string,
    plan: CapturePlan,
  ): Promise<CaptureOutcome> => {
    if (closed) {
      return { ok: false, code: 'browser-failed', message: 'capture session is already closed' };
    }
    const outDir = fixtureOutDir(options.outDir, fixture, plan);
    try {
      await ensureOutDir(outDir);
      const ctx: CaptureFixtureContext = {
        browser,
        baseUrl: server.url,
        outDir,
        repoRoot: options.repoRoot,
        docxWindow: pageDocxWindow(),
        ...(options.stableTimeoutMs === undefined
          ? {}
          : { stableTimeoutMs: options.stableTimeoutMs }),
        ...(options.openTimeoutMs === undefined ? {} : { openTimeoutMs: options.openTimeoutMs }),
      };
      const result = await captureFixture(ctx, fixture, plan);
      await writeRuntime(outDir, result.runtime);
      return {
        ok: true,
        captures: result.captures,
        runtime: result.runtime,
        structure: result.structure,
      };
    } catch (error) {
      const failure = asCaptureError(error, 'browser-failed', `capture of "${fixture}" failed`);
      return { ok: false, code: failure.code, message: failure.message };
    }
  };

  return { capture, close };
}

/** Каталог вывода фикстуры — нужен CLI, чтобы найти `runtime.json`. */
export function captureOutDir(
  outDir: string,
  fixture: string,
  plan: CapturePlan,
): string {
  return fixtureOutDir(outDir, fixture, plan);
}

/** Создать каталог вывода заранее — для CLI, который пишет отчёт рядом. */
export { ensureOutDir };
