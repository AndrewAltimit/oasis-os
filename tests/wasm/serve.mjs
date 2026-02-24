// Minimal HTTP server for the WASM demo.
//
// Serves www/ and pkg/ from the repo root with correct MIME types
// (wasm needs application/wasm for streaming compilation).

import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { resolve, extname } from "node:path";

const MIME = {
  ".html": "text/html",
  ".css": "text/css",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".wasm": "application/wasm",
  ".png": "image/png",
  ".json": "application/json",
};

/**
 * Start a static file server rooted at the repository root.
 * Returns { server, port, close() }.
 */
export async function startServer(port = 0) {
  const root = resolve(new URL(".", import.meta.url).pathname, "../..");

  const server = createServer(async (req, res) => {
    let url = new URL(req.url, `http://localhost`).pathname;
    if (url === "/") url = "/www/index.html";

    const filePath = resolve(root, `.${url}`);

    // Security: don't escape root.
    if (!filePath.startsWith(root)) {
      res.writeHead(403);
      res.end("Forbidden");
      return;
    }

    try {
      const info = await stat(filePath);
      if (!info.isFile()) throw new Error("not a file");
    } catch {
      res.writeHead(404);
      res.end("Not found");
      return;
    }

    const ext = extname(filePath);
    const mime = MIME[ext] || "application/octet-stream";
    const data = await readFile(filePath);

    res.writeHead(200, {
      "Content-Type": mime,
      "Cross-Origin-Opener-Policy": "same-origin",
      "Cross-Origin-Embedder-Policy": "require-corp",
    });
    res.end(data);
  });

  return new Promise((ok) => {
    server.listen(port, "127.0.0.1", () => {
      const addr = server.address();
      ok({
        server,
        port: addr.port,
        url: `http://127.0.0.1:${addr.port}`,
        close: () => server.close(),
      });
    });
  });
}

// Run standalone if invoked directly.
if (process.argv[1] === new URL("", import.meta.url).pathname) {
  const srv = await startServer(8080);
  console.log(`Serving at ${srv.url}`);
}
