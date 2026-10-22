import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri очікує фіксований порт і не потребує відкриття браузера.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "es2022",
    chunkSizeWarningLimit: 1500,
  },
});
