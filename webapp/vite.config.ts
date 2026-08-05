import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The build output is embedded into the server binary by rust_embed, so assets
// must be self-contained and content-hashed: the server caches them for a year
// and serves index.html no-cache.
export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  server: {
    // In development the client runs on Vite while the API stays on the server,
    // so same-origin calls keep working without a CORS allow-list.
    proxy: {
      "/api": "http://127.0.0.1:4533",
    },
  },
});
