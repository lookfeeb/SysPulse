import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 5174 }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    target: "chrome105",
    minify: "esbuild",
    sourcemap: false,
    rollupOptions: {
      input: {
        main: path.resolve(__dirname, "index.html"),
        overlay: path.resolve(__dirname, "overlay/index.html"),
        "overlay-tooltip": path.resolve(__dirname, "overlay-tooltip/index.html"),
      },
      output: {
        manualChunks: {
          react: ["react", "react-dom", "react-router-dom"],
          antd: ["antd"],
        },
      },
    },
  },
});
