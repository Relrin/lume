import { fileURLToPath } from "node:url";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

/**
 * Whether this is a build for testing. Such a build also contains the harness page,
 * with the recordings and the test font it needs, and source maps. A build that is
 * shipped contains neither: `LUME_HARNESS=1` asks for the first kind.
 */
const forTesting = process.env["LUME_HARNESS"] === "1";

const page = (name: string) => fileURLToPath(new URL(name, import.meta.url));

/** The pages every build contains: the terminal window's and the settings window's. */
const pages = { index: page("index.html"), settings: page("settings.html") };

// Tauri loads this dev server at a fixed address (tauri.conf.json, build.devUrl).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "localhost",
  },
  build: {
    target: "es2023",
    sourcemap: forTesting,
    rolldownOptions: {
      input: forTesting ? { ...pages, harness: page("harness.html") } : pages,
    },
  },
});
