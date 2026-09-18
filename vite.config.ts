import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import os from "os";
import path from "path";

const now = new Date();
const pad = (n: number) => n.toString().padStart(2, "0");
const buildTimestamp = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}`;
const buildNumber = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}.${pad(now.getHours())}${pad(now.getMinutes())}`;

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [svelte()],
  cacheDir: path.join(os.tmpdir(), "trackhelm-vite-cache"),
  define: {
    __BUILD_TIMESTAMP__: JSON.stringify(buildTimestamp),
    __BUILD_NUMBER__: JSON.stringify(buildNumber),
  },
  esbuild: {
    target: "esnext",
  },
  optimizeDeps: {
    esbuildOptions: {
      target: "esnext",
    },
  },
  build: {
    target: "esnext",
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  // prevent vite from obscuring rust errors
  clearScreen: false,
  // tauri expects a fixed port, fail if that port is not available
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // tell vite to ignore watching src-tauri
      ignored: ["**/src-tauri/**"],
    },
  },
});
