import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Tauri loads this dev server at a fixed address (tauri.conf.json, build.devUrl).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "localhost",
  },
  build: {
    target: "es2023",
    sourcemap: true,
  },
});
