import { defineConfig, type Plugin } from "vite";
import solid from "vite-plugin-solid";
import { viteSingleFile } from "vite-plugin-singlefile";
import { readFileSync, writeFileSync, existsSync, readdirSync } from "node:fs";
import { resolve, join } from "node:path";
import { brotliCompressSync, constants as zlibConstants } from "node:zlib";
import { request as httpRequest } from "node:http";

const wasmPath = resolve(
  __dirname,
  "../../crates/browser/pkg/blit_browser_bg.wasm",
);
const snippetsDir = resolve(__dirname, "../../crates/browser/pkg/snippets");
const isDev =
  process.env.NODE_ENV !== "production" && !process.argv.includes("build");
const devBase = process.env.BLIT_DEV_BASE || "/";

export default defineConfig({
  base: isDev ? devBase : "/",
  plugins: [
    solid(),
    // Only inline everything into a single HTML file for production builds.
    !isDev && viteSingleFile(),
    {
      name: "inline-wasm",
      resolveId(id) {
        if (id === "virtual:blit-wasm") return "\0virtual:blit-wasm";
      },
      load(id) {
        if (id !== "\0virtual:blit-wasm") return;
        if (isDev) {
          // In dev, use a URL import so Vite serves the file directly.
          // Prefix with the base so it works behind a reverse proxy.
          const base = devBase.endsWith("/") ? devBase.slice(0, -1) : devBase;
          return `export default "${base}/@fs${wasmPath}";`;
        }
        const wasm = readFileSync(wasmPath);
        const b64 = wasm.toString("base64");
        return `
const b64 = ${JSON.stringify(b64)};
const bin = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
export default bin.buffer;
`;
      },
    },
    {
      name: "resolve-blit-snippets",
      resolveId(id, importer) {
        const match = id.match(/\.\/snippets\/blit-browser-[^/]+\/(.*)/);
        if (match && importer && existsSync(snippetsDir)) {
          const file = match[1];
          for (const dir of readdirSync(snippetsDir)) {
            const candidate = join(snippetsDir, dir, file);
            if (existsSync(candidate)) return candidate;
          }
        }
      },
    },
    // Dev: handle reverse-proxy path rewriting and blit WS proxying.
    isDev && {
      name: "blit-dev-proxy",
      configureServer(server) {
        const base = devBase.endsWith("/") ? devBase : devBase + "/";
        const needsRewrite = base !== "/";
        const prefix = base.slice(0, -1); // e.g. "/vt2"

        const gwHost = process.env.VITE_BLIT_GATEWAY || `localhost:${process.env.BLIT_DEV_GW_PORT || "3266"}`;
        const [gwHostname, gwPort] = gwHost.includes(":")
          ? [gwHost.slice(0, gwHost.lastIndexOf(":")), gwHost.slice(gwHost.lastIndexOf(":") + 1)]
          : [gwHost, "80"];

        function proxyWsToGateway(
          req: import("node:http").IncomingMessage,
          socket: import("node:stream").Duplex,
          gwPath: string,
        ) {
          const proxyReq = httpRequest({
            hostname: gwHostname,
            port: parseInt(gwPort),
            path: gwPath,
            method: req.method,
            headers: req.headers,
          });
          proxyReq.on("upgrade", (_res, proxySocket, proxyHead) => {
            socket.write(
              "HTTP/1.1 101 Switching Protocols\r\n" +
              "Upgrade: websocket\r\n" +
              "Connection: Upgrade\r\n" +
              `Sec-WebSocket-Accept: ${_res.headers["sec-websocket-accept"]}\r\n` +
              (_res.headers["sec-websocket-protocol"]
                ? `Sec-WebSocket-Protocol: ${_res.headers["sec-websocket-protocol"]}\r\n`
                : "") +
              "\r\n",
            );
            if (proxyHead.length) socket.write(proxyHead);
            proxySocket.pipe(socket);
            socket.pipe(proxySocket);
          });
          proxyReq.on("error", () => socket.destroy());
          proxyReq.end();
        }

        // HTTP: re-add the stripped base prefix so vite's router matches.
        if (needsRewrite) {
          server.middlewares.use((req, _res, next) => {
            if (req.url && !req.url.startsWith(base)) {
              req.url = prefix + req.url;
            }
            next();
          });
        }

        // WS upgrades bypass Connect middleware, so rewrite + route here.
        server.httpServer?.on("upgrade", (req, socket, head) => {
          const raw = req.url || "/";

          // Strip base prefix (or proxy-stripped form) to get the inner path.
          let inner = raw;
          if (needsRewrite && raw.startsWith(base)) {
            inner = raw.slice(prefix.length);
          } else if (needsRewrite && !raw.startsWith(base)) {
            // Proxy already stripped the prefix — raw IS the inner path.
            inner = raw;
            // Rewrite req.url so vite's own handlers see the base prefix.
            req.url = prefix + raw;
          }

          // Vite internal paths (HMR, client, etc.).
          // Vite HMR connects to the base path with ?token=… query.
          if (inner.startsWith("/__") || inner.startsWith("/@")) {
            return; // let vite handle it
          }
          const url = new URL(inner, "http://localhost");
          if (url.searchParams.has("token")) {
            return; // vite HMR
          }

          // /config WS → gateway (strip base prefix).
          // Everything else (blit transport) → gateway (keep full path).
          proxyWsToGateway(req, socket, inner);
        });
      },
    },
    !isDev && {
      name: "brotli-html",
      closeBundle() {
        const htmlPath = resolve(__dirname, "dist/index.html");
        if (existsSync(htmlPath)) {
          const html = readFileSync(htmlPath);
          const compressed = brotliCompressSync(html, {
            params: {
              [zlibConstants.BROTLI_PARAM_QUALITY]:
                zlibConstants.BROTLI_MAX_QUALITY,
            },
          });
          writeFileSync(htmlPath + ".br", compressed);
        }
      },
    },
  ].filter(Boolean),
  resolve: {
    alias: {
      "@blit-sh/browser": resolve(
        __dirname,
        "../../crates/browser/pkg/blit_browser.js",
      ),
    },
    dedupe: ["solid-js"],
  },
  server: {
    port: parseInt(process.env.BLIT_DEV_UI_PORT || "3265"),
    host: "0.0.0.0",
    fs: {
      // Allow serving the WASM file from outside the ui directory.
      allow: [resolve(__dirname, "../..")],
    },
    proxy: isDev
      ? (() => {
          const gw = `http://${process.env.VITE_BLIT_GATEWAY || `localhost:${process.env.BLIT_DEV_GW_PORT || "3266"}`}`;
          // After the base-rewrite middleware, requests arrive with the
          // base prefix prepended.  Match the prefixed paths and rewrite
          // back to the bare path before forwarding to the gateway.
          const b = devBase === "/" ? "" : devBase.replace(/\/$/, "");
          return {
            [`${b}/config`]: { target: gw, rewrite: (p: string) => p.slice(b.length) },
            [`${b}/fonts`]: { target: gw, rewrite: (p: string) => p.slice(b.length) },
            [`${b}/font`]:  { target: gw, rewrite: (p: string) => p.slice(b.length) },
          };
        })()
      : undefined,
  },
  build: {
    outDir: resolve(__dirname, "dist"),
    target: "es2020",
  },
});
