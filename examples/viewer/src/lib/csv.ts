/** Разбор CSV с кавычками, CRLF и экранированными кавычками, без eval и внешних сервисов. */
export function parseCsv(source: string, delimiter?: string): string[][] {
  const first = source.split(/\r?\n/, 1)[0] ?? '';
  const separator = delimiter ?? ((first.match(/;/g)?.length ?? 0) > (first.match(/,/g)?.length ?? 0) ? ';' : ',');
  const rows: string[][] = [];
  let row: string[] = [];
  let field = '';
  let quoted = false;
  const pushField = (): void => {
    row.push(field);
    field = '';
  };
  const pushRow = (): void => {
    pushField();
    if (row.some((cell) => cell.trim() !== '')) rows.push(row);
    row = [];
  };
  for (let index = 0; index < source.length; index += 1) {
    const char = source[index];
    if (quoted) {
      if (char === '"' && source[index + 1] === '"') {
        field += '"';
        index += 1;
      } else if (char === '"') quoted = false;
      else field += char;
    } else if (char === '"' && field.trim() === '') quoted = true;
    else if (char === separator) pushField();
    else if (char === '\n') pushRow();
    else if (char !== '\r') field += char;
  }
  pushRow();
  return rows;
}

/** Первая строка — имена полей слияния; пустые имена получают стабильный ColumnN. */
export function csvRecords(source: string): Record<string, string>[] {
  const [headers = [], ...rows] = parseCsv(source);
  const names = headers.map((name, index) => name.trim() || `Column${index + 1}`);
  return rows.map((row) => Object.fromEntries(names.map((name, index) => [name, row[index] ?? ''])));
}
