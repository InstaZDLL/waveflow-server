import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The build output is embedded into the server binary by rust_embed, so assets
// must be self-contained and content-hashed: the server caches them for a year
// and serves index.html no-cache.
export default defineConfig({
  plugins: [react()],
  // Under Home Assistant's ingress the client is served below a prefix
  // (RFC-030), so the stylesheet must reach its fonts, and a chunk its
  // neighbours, relative to themselves. The shell keeps rooted references,
  // which the server anchors to the request's root as it serves it, and which
  // `vite preview` and the end-to-end suite serve as they are.
  experimental: {
    renderBuiltUrl(filename, { hostType }) {
      return hostType === "html" ? `/${filename}` : { relative: true };
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  server: {
    // In development the client runs on Vite while the API stays on the server,
    // so same-origin calls keep working without a CORS allow-list.
    proxy: {
      "/api": "http://127.0.0.1:4533",
      // The probe the source offer is read from (AGPL-3.0 §13).
      "/health": "http://127.0.0.1:4533",
      // The share address answers the page to a browser and the share to the
      // page (RFC-020): in development the page is Vite's to serve, and only
      // the JSON request goes through to the server.
      "/share": {
        target: "http://127.0.0.1:4533",
        bypass: (request) =>
          request.headers.accept?.includes("text/html")
            ? "/index.html"
            : undefined,
      },
    },
  },
});
