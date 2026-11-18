export type FormulaValue = number | string | boolean | number[];
export type ReferenceResolver = (reference: string) => number;
export type RangeResolver = (reference: string) => number[];

interface Token {
  type: 'number' | 'string' | 'identifier' | 'reference' | 'operator' | 'left' | 'right' | 'separator' | 'end';
  value: string;
}

function tokenize(source: string): Token[] {
  const tokens: Token[] = [];
  let index = 0;
  while (index < source.length) {
    const char = source[index] ?? '';
    if (/\s/.test(char)) {
      index += 1;
      continue;
    }
    if (char === '"') {
      let value = '';
      index += 1;
      while (index < source.length) {
        const next = source[index] ?? '';
        if (next === '"' && source[index + 1] === '"') {
          value += '"';
          index += 2;
        } else if (next === '"') {
          index += 1;
          break;
        } else {
          value += next;
          index += 1;
        }
      }
      tokens.push({ type: 'string', value });
      continue;
    }
    const number = /^(?:\d+(?:\.\d*)?|\.\d+)(?:[Ee][+-]?\d+)?/.exec(source.slice(index));
    if (number) {
      tokens.push({ type: 'number', value: number[0] });
      index += number[0].length;
      continue;
    }
    if (char === "'") {
      const start = index;
      index += 1;
      while (index < source.length) {
        if (source[index] === "'" && source[index + 1] === "'") index += 2;
        else if (source[index] === "'") {
          index += 1;
          break;
        } else index += 1;
      }
      if (source[index] === '!') {
        index += 1;
        const reference = /^\$?[A-Za-z]{1,3}\$?[1-9]\d*(?::\$?[A-Za-z]{1,3}\$?[1-9]\d*)?/.exec(source.slice(index));
        if (!reference) throw new Error('некорректная ссылка после имени листа');
        index += reference[0].length;
        tokens.push({ type: 'reference', value: source.slice(start, index) });
        continue;
      }
      throw new Error('некорректное имя листа в формуле');
    }
    if (/[A-Za-z_$]/.test(char)) {
      const word = /^[A-Za-z0-9_.$!:]/.exec(source.slice(index));
      void word;
      const rest = source.slice(index);
      const reference = /^(?:(?:[A-Za-z_][\w.]*)!)?\$?[A-Za-z]{1,3}\$?[1-9]\d*(?::\$?[A-Za-z]{1,3}\$?[1-9]\d*)?/.exec(rest);
      const identifier = /^[A-Za-z_][\w.]*/.exec(rest);
      if (reference && (!identifier || reference[0].length >= identifier[0].length)) {
        tokens.push({ type: 'reference', value: reference[0] });
        index += reference[0].length;
      } else if (identifier) {
        tokens.push({ type: 'identifier', value: identifier[0] });
        index += identifier[0].length;
      } else {
        throw new Error(`неожиданный символ «${char}» в формуле`);
      }
      continue;
    }
    if (char === '(' || char === ')') {
      tokens.push({ type: char === '(' ? 'left' : 'right', value: char });
      index += 1;
      continue;
    }
    if (char === ',' || char === ';') {
      tokens.push({ type: 'separator', value: char });
      index += 1;
      continue;
    }
    const operator = /^(?:<>|<=|>=|[-+*\/^&=<>%])/.exec(source.slice(index));
    if (operator) {
      tokens.push({ type: 'operator', value: operator[0] });
      index += operator[0].length;
      continue;
    }
    throw new Error(`неподдерживаемый символ «${char}» в формуле`);
  }
  tokens.push({ type: 'end', value: '' });
  return tokens;
}

function localReference(reference: string): string {
  return reference.replace(/^.*!/, '').replace(/\$/g, '');
}

function numeric(value: FormulaValue): number {
  if (Array.isArray(value)) throw new Error('диапазон нельзя использовать как одиночное число');
  if (typeof value === 'number') return value;
  if (typeof value === 'boolean') return value ? 1 : 0;
  const parsed = Number(value.replace(/\s/g, '').replace(',', '.'));
  if (!Number.isFinite(parsed)) throw new Error(`«${value}» не является числом`);
  return parsed;
}

function flatten(values: FormulaValue[]): number[] {
  return values.flatMap((value) => Array.isArray(value) ? value : [numeric(value)]);
}

function pmt(rate: number, periods: number, present: number, future = 0, type = 0): number {
  if (periods === 0) throw new Error('PMT: nper не должен быть нулевым');
  if (rate === 0) return -(present + future) / periods;
  const factor = (1 + rate) ** periods;
  return -(present * factor + future) * rate / ((1 + rate * type) * (factor - 1));
}

function fv(rate: number, periods: number, payment: number, present = 0, type = 0): number {
  if (rate === 0) return -(present + payment * periods);
  const factor = (1 + rate) ** periods;
  return -(present * factor + payment * (1 + rate * type) * (factor - 1) / rate);
}

function pv(rate: number, periods: number, payment: number, future = 0, type = 0): number {
  if (rate === 0) return -(future + payment * periods);
  const factor = (1 + rate) ** periods;
  return -(future + payment * (1 + rate * type) * (factor - 1) / rate) / factor;
}

class Parser {
  private index = 0;

  constructor(
    private readonly tokens: Token[],
    private readonly reference: ReferenceResolver,
    private readonly range: RangeResolver,
  ) {}

  parse(): FormulaValue {
    const value = this.comparison();
    if (this.current().type !== 'end') throw new Error(`неожиданный токен «${this.current().value}»`);
    return value;
  }

  private current(): Token {
    return this.tokens[this.index] ?? { type: 'end', value: '' };
  }

  private take(): Token {
    const token = this.current();
    this.index += 1;
    return token;
  }

  private accept(value: string): boolean {
    if (this.current().value !== value) return false;
    this.index += 1;
    return true;
  }

  private expect(type: Token['type'], message: string): Token {
    if (this.current().type !== type) throw new Error(message);
    return this.take();
  }

  private comparison(): FormulaValue {
    let left = this.concatenation();
    const operator = this.current().value;
    if (['=', '<>', '<', '>', '<=', '>='].includes(operator)) {
      this.take();
      const right = this.concatenation();
      const a = Array.isArray(left) ? NaN : typeof left === 'string' ? left : numeric(left);
      const b = Array.isArray(right) ? NaN : typeof right === 'string' ? right : numeric(right);
      if (typeof a === 'number' && Number.isNaN(a) || typeof b === 'number' && Number.isNaN(b)) {
        throw new Error('диапазон нельзя сравнивать');
      }
      if (operator === '=') return a === b;
      if (operator === '<>') return a !== b;
      if (operator === '<') return a < b;
      if (operator === '>') return a > b;
      if (operator === '<=') return a <= b;
      return a >= b;
    }
    return left;
  }

  private concatenation(): FormulaValue {
    let left = this.addition();
    while (this.accept('&')) {
      const right = this.addition();
      if (Array.isArray(left) || Array.isArray(right)) throw new Error('диапазон нельзя объединять со строкой');
      left = `${String(left)}${String(right)}`;
    }
    return left;
  }

  private addition(): FormulaValue {
    let left = this.multiplication();
    while (this.current().value === '+' || this.current().value === '-') {
      const operator = this.take().value;
      const right = this.multiplication();
      left = operator === '+' ? numeric(left) + numeric(right) : numeric(left) - numeric(right);
    }
    return left;
  }

  private multiplication(): FormulaValue {
    let left = this.power();
    while (this.current().value === '*' || this.current().value === '/') {
      const operator = this.take().value;
      const right = this.power();
      if (operator === '/' && numeric(right) === 0) throw new Error('деление на ноль');
      left = operator === '*' ? numeric(left) * numeric(right) : numeric(left) / numeric(right);
    }
    return left;
  }

  private power(): FormulaValue {
    let left = this.unary();
    if (this.accept('^')) left = numeric(left) ** numeric(this.power());
    return left;
  }

  private unary(): FormulaValue {
    if (this.accept('+')) return this.unary();
    if (this.accept('-')) return -numeric(this.unary());
    return this.postfix();
  }

  private postfix(): FormulaValue {
    let value = this.primary();
    while (this.accept('%')) value = numeric(value) / 100;
    return value;
  }

  private primary(): FormulaValue {
    const token = this.current();
    if (token.type === 'number') {
      this.take();
      return Number(token.value);
    }
    if (token.type === 'string') {
      this.take();
      return token.value;
    }
    if (token.type === 'reference') {
      this.take();
      const reference = localReference(token.value);
      return reference.includes(':') ? this.range(reference) : this.reference(reference);
    }
    if (token.type === 'identifier') {
      this.take();
      if (token.value.toUpperCase() === 'TRUE') return true;
      if (token.value.toUpperCase() === 'FALSE') return false;
      this.expect('left', `для функции ${token.value} ожидалась «(»`);
      const args: FormulaValue[] = [];
      if (this.current().type !== 'right') {
        do {
          args.push(this.comparison());
        } while (this.accept(',') || this.accept(';'));
      }
      this.expect('right', `для функции ${token.value} ожидалась «)»`);
      return this.call(token.value.toUpperCase(), args);
    }
    if (this.accept('(')) {
      const value = this.comparison();
      this.expect('right', 'ожидалась закрывающая скобка');
      return value;
    }
    throw new Error(`неожиданный токен «${token.value || 'конец формулы'}»`);
  }

  private call(name: string, args: FormulaValue[]): FormulaValue {
    if (name === 'SUM') return flatten(args).reduce((sum, value) => sum + value, 0);
    if (name === 'AVERAGE' || name === 'СРЗНАЧ') {
      const values = flatten(args);
      if (values.length === 0) throw new Error('AVERAGE: пустой список аргументов');
      return values.reduce((sum, value) => sum + value, 0) / values.length;
    }
    if (name === 'MIN') return Math.min(...flatten(args));
    if (name === 'MAX') return Math.max(...flatten(args));
    if (name === 'COUNT') return flatten(args).length;
    if (name === 'ABS') return Math.abs(numeric(args[0] ?? 0));
    if (name === 'ROUND') {
      const digits = numeric(args[1] ?? 0);
      const factor = 10 ** digits;
      return Math.round(numeric(args[0] ?? 0) * factor) / factor;
    }
    if (name === 'PMT') return pmt(numeric(args[0] ?? 0), numeric(args[1] ?? 0), numeric(args[2] ?? 0), numeric(args[3] ?? 0), numeric(args[4] ?? 0));
    if (name === 'FV') return fv(numeric(args[0] ?? 0), numeric(args[1] ?? 0), numeric(args[2] ?? 0), numeric(args[3] ?? 0), numeric(args[4] ?? 0));
    if (name === 'PV') return pv(numeric(args[0] ?? 0), numeric(args[1] ?? 0), numeric(args[2] ?? 0), numeric(args[3] ?? 0), numeric(args[4] ?? 0));
    throw new Error(`функция ${name} не поддерживается локальным вычислителем`);
  }
}

export function evaluateFormula(
  source: string,
  reference: ReferenceResolver,
  range: RangeResolver,
): FormulaValue {
  const formula = source.trim().replace(/^=/, '');
  if (!formula) throw new Error('пустая формула');
  const value = new Parser(tokenize(formula), reference, range).parse();
  if (Array.isArray(value)) throw new Error('результатом формулы не может быть диапазон');
  return value;
}
