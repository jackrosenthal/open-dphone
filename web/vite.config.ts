import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// `pnpm dev` proxies the API to a phone or to the simulator
// (DPHONE=http://10.0.0.5 pnpm dev; the default is the host example server).
const target = process.env.DPHONE ?? "http://127.0.0.1:8480";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  build: {
    target: "es2022",
    sourcemap: false,
    cssCodeSplit: false,
    chunkSizeWarningLimit: 700,
  },
  server: {
    proxy: {
      "^/dphone\\.v1\\..*": { target, changeOrigin: false },
      "/upload": { target, changeOrigin: false },
    },
  },
});
