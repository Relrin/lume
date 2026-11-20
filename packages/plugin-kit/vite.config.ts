// Builds the kit into modules a plugin's own bundler reads: everything, and the way to
// the plugin's code alone for a view that is not written with React. React is left to
// the plugin, which brings its own copy.
import { fileURLToPath } from "node:url";

import { defineConfig } from "vite";

const source = (name: string) => fileURLToPath(new URL(`src/${name}`, import.meta.url));

export default defineConfig({
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2023",
    minify: false,
    sourcemap: false,
    lib: {
      entry: { index: source("index.ts"), bridge: source("bridge.ts") },
      formats: ["es"],
    },
    rolldownOptions: {
      external: ["react", "react/jsx-runtime", "react-dom"],
      output: { chunkFileNames: "[name]-shared.js" },
    },
  },
});
