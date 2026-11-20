import { fileURLToPath } from "node:url";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, type Plugin } from "vite";

/**
 * Whether this is a build for testing. Such a build also contains the harness page,
 * with the recordings and the test font it needs, and source maps. A build that is
 * shipped contains neither: `LUME_HARNESS=1` asks for the first kind.
 */
const forTesting = process.env["LUME_HARNESS"] === "1";

const page = (name: string) => fileURLToPath(new URL(name, import.meta.url));

/** The pages every build contains: the terminal window's and the settings window's. */
const pages = { index: page("index.html"), settings: page("settings.html") };

/** The call sonner starts with: it writes its stylesheet into the page. */
const SONNER_INSERTS_CSS = /^__insertCSS\(".*"\);?$/m;

/**
 * Keeps sonner from writing its stylesheet into the page as it starts.
 *
 * The application's content security policy lets a page use only the stylesheets it
 * was built with, so a style element made by a script is refused, with a complaint in
 * the console. The same styles are part of the page's own stylesheet instead (see the
 * top of src/styles.css). A build fails if a later sonner no longer starts this way,
 * so that a change of its is noticed here and not by the policy.
 *
 * The dev server does the same, so that what is seen while working is what is built:
 * styles a script adds would come after the page's own and have the last word over
 * them. It is told not to prepare sonner ahead (see `optimizeDeps` below), since what
 * is prepared ahead does not pass through here.
 */
function sonnerStylesFromThePage(): Plugin {
  return {
    name: "lume:sonner-styles",
    enforce: "pre",
    transform(code, id) {
      if (!/[\\/]sonner[\\/]dist[\\/]index\.m?js$/.test(id.split("?")[0] ?? id)) return null;
      if (!SONNER_INSERTS_CSS.test(code)) {
        this.error("sonner no longer adds its styles the way this build takes out");
      }
      return { code: code.replace(SONNER_INSERTS_CSS, ""), map: null };
    },
  };
}

// Tauri loads this dev server at a fixed address (tauri.conf.json, build.devUrl).
export default defineConfig({
  plugins: [sonnerStylesFromThePage(), react(), tailwindcss()],
  clearScreen: false,
  optimizeDeps: { exclude: ["sonner"] },
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
