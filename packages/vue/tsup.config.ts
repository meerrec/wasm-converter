import { defineConfig } from 'tsup';

export default defineConfig({
  entry: {
    index: 'src/index.ts',
  },
  format: ['esm', 'cjs'],
  dts: true,
  sourcemap: true,
  clean: true,
  target: 'es2022',
  splitting: false,
  treeshake: true,
  // Vue — peerDependency, в бандл обёртки не входит; @meerrec/core tsup
  // выносит сам как зависимость из package.json.
  external: ['vue'],
});
