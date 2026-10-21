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
  // React — peerDependency, @meerrec/core — обычная зависимость: в бандл
  // обёртки они не входят (в core их забирает tsup из `dependencies` сам).
  external: ['react', 'react-dom', 'react/jsx-runtime'],
});
