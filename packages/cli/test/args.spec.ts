import { describe, expect, it } from 'vitest';

import { defaultWorkers, parseArgs } from '../src/args.js';
import { CliError } from '../src/errors.js';

describe('parseArgs', () => {
  it('без аргументов показывает справку', () => {
    expect(parseArgs([])).toEqual({ kind: 'help' });
  });

  it('разбирает convert с -o и форматом по расширению', () => {
    const command = parseArgs(['convert', 'report.docx', '-o', 'report.pdf']);
    expect(command).toEqual({
      kind: 'convert',
      input: 'report.docx',
      out: 'report.pdf',
      format: null,
      force: false,
      pdf: {},
    });
  });

  it('принимает явный --format и опции PDF', () => {
    const command = parseArgs([
      'convert',
      'book.xlsx',
      '-o',
      'book.out',
      '--format',
      'pdf',
      '--page-size',
      'a3',
      '--orientation',
      'landscape',
      '--scale',
      '0.8',
      '--margins',
      '10,12.5,10,12.5',
      '--sheet',
      '2',
    ]);
    if (command.kind !== 'convert') throw new Error('expected convert');
    expect(command.format).toBe('pdf');
    expect(command.pdf).toEqual({
      pageSize: 'A3',
      orientation: 'landscape',
      scale: 0.8,
      margins: { top: 10, right: 12.5, bottom: 10, left: 12.5 },
      sheetIndex: 2,
    });
  });

  it('разбирает batch с воркерами и --force', () => {
    const command = parseArgs(['batch', './in/*.xlsx', './more', '--out', 'out', '--workers', '4', '--force']);
    expect(command).toEqual({
      kind: 'batch',
      inputs: ['./in/*.xlsx', './more'],
      outDir: 'out',
      format: 'pdf',
      workers: 4,
      force: true,
      quiet: false,
      pdf: {},
    });
  });

  it('по умолчанию в batch — pdf и число воркеров по ядрам', () => {
    const command = parseArgs(['batch', 'in', '--out', 'out']);
    if (command.kind !== 'batch') throw new Error('expected batch');
    expect(command.format).toBe('pdf');
    expect(command.workers).toBe(defaultWorkers());
    expect(command.workers).toBeGreaterThanOrEqual(1);
  });

  it('поддерживает --option=value', () => {
    const command = parseArgs(['batch', 'in', '--out=out', '--workers=2', '--format=md']);
    if (command.kind !== 'batch') throw new Error('expected batch');
    expect(command.outDir).toBe('out');
    expect(command.workers).toBe(2);
    expect(command.format).toBe('md');
  });

  it('после -- аргументы не считаются опциями', () => {
    const command = parseArgs(['convert', '-o', 'out.pdf', '--', '-weird.docx']);
    if (command.kind !== 'convert') throw new Error('expected convert');
    expect(command.input).toBe('-weird.docx');
  });

  it('ругается на неизвестный флаг', () => {
    expect(() => parseArgs(['convert', 'a.docx', '-o', 'a.pdf', '--turbo'])).toThrow(CliError);
  });

  it('ругается на неизвестный формат', () => {
    expect(() => parseArgs(['convert', 'a.docx', '-o', 'a.pdf', '--format', 'docx'])).toThrow(/unknown format/);
  });

  it('требует -o у convert и --out у batch', () => {
    expect(() => parseArgs(['convert', 'a.docx'])).toThrow(/--out/);
    expect(() => parseArgs(['batch', 'in'])).toThrow(/--out/);
  });

  it('ругается на несколько входов convert и на пустой batch', () => {
    expect(() => parseArgs(['convert', 'a.docx', 'b.docx', '-o', 'out.pdf'])).toThrow(/one input file/);
    expect(() => parseArgs(['batch', '--out', 'out'])).toThrow(/at least one input/);
  });

  it('проверяет значения опций', () => {
    expect(() => parseArgs(['batch', 'in', '--out', 'out', '--workers', '0'])).toThrow(/at least 1/);
    expect(() => parseArgs(['batch', 'in', '--out', 'out', '--workers', 'x'])).toThrow(/non-negative integer/);
    expect(() => parseArgs(['convert', 'a.xlsx', '-o', 'a.pdf', '--page-size', 'A5'])).toThrow(/unknown page size/);
    expect(() => parseArgs(['convert', 'a.xlsx', '-o', 'a.pdf', '--margins', '1,2,3'])).toThrow(/four comma-separated/);
    expect(() => parseArgs(['convert', 'a.xlsx', '-o', 'a.pdf', '--scale', '-1'])).toThrow(/positive number/);
  });

  it('показывает справку по help в подкоманде', () => {
    expect(parseArgs(['convert', '--help'])).toEqual({ kind: 'help' });
    expect(parseArgs(['help'])).toEqual({ kind: 'help' });
    expect(parseArgs(['--version'])).toEqual({ kind: 'version' });
  });
});
