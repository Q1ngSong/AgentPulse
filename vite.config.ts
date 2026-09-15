import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  root: "src",
  plugins: [react()],
  base: "./",
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: path.resolve(__dirname, "src/index.html"),
        overlay: path.resolve(__dirname, "src/overlay.html"),
      },
    },
  },
  server: { port: 3000, strictPort: true },
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_"],
});
