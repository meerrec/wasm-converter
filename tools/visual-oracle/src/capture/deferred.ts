// Отложенный промис: `Promise.withResolvers` появился только в Node 22, а
// пакет объявляет `engines.node >= 20`, поэтому резолверы достаются вручную.

/** Промис вместе с его резолверами. */
export interface Deferred<T> {
  readonly promise: Promise<T>;
  resolve(value: T): void;
  reject(reason: unknown): void;
}

/** Создать отложенный промис. */
export function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}
