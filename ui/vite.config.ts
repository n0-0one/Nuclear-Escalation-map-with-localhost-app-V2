import { defineConfig } from "vite";
import preact from "@preact/preset-vite";

// Relative asset paths so the same build works inside the desktop app.
export default defineConfig({
  base: "./",
  plugins: [preact()],
  build: { target: "es2022", sourcemap: false, assetsInlineLimit: 0, chunkSizeWarningLimit: 400 },
  server: {
    port: 5173,
    strictPort: true,
    proxy: { "/api": "http://127.0.0.1:8777", "/pack": "http://127.0.0.1:8777" },
  },
});
