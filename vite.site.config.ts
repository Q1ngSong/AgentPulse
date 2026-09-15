import path from "node:path";
import { defineConfig } from "vite";

// 网站单独构建，不进入桌面 App 的 frontendDist。
export default defineConfig({
  root: "site",
  base: "/AgentPulse/",
  build: {
    outDir: "../dist-site",
    emptyOutDir: true,
    rollupOptions: { input: { home: path.resolve(__dirname, "site/index.html"), guide: path.resolve(__dirname, "site/guide/index.html") } },
  },
  server: { host: "127.0.0.1", port: 3420, strictPort: true, fs: { allow: [path.resolve(__dirname)] } },
});
