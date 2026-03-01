#!/usr/bin/env node
// Development server for OASIS_OS WASM with CORS proxy.
//
// Serves www/ and pkg/ with correct MIME types, plus a /cors-proxy/ route
// that fetches remote URLs (e.g. archive.org MP4s) and forwards them
// with CORS headers.  This enables the software video decode path for
// browsers that lack H.264 codecs (Firefox snap, Playwright Chromium).
//
// Usage:
//   node scripts/serve-wasm.mjs              # port 8080
//   node scripts/serve-wasm.mjs --port 3000  # custom port

import { createServer, request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
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
  ".mp4": "video/mp4",
  ".toml": "text/plain",
};

const ROOT = resolve(new URL(".", import.meta.url).pathname, "..");
const PORT = parseInt(process.argv.find((_, i, a) => a[i - 1] === "--port") || "8080", 10);

const server = createServer(async (req, res) => {
  const parsed = new URL(req.url, "http://localhost");
  let pathname = parsed.pathname;

  // --- CORS proxy route ---
  if (pathname === "/cors-proxy") {
    const targetUrl = parsed.searchParams.get("url");
    if (!targetUrl) {
      res.writeHead(400, corsHeaders("text/plain"));
      res.end("Missing ?url= parameter");
      return;
    }
    return proxyFetch(targetUrl, res);
  }

  // --- Static file serving ---
  if (pathname === "/") pathname = "/www/index.html";

  const filePath = resolve(ROOT, `.${pathname}`);
  if (!filePath.startsWith(ROOT)) {
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
  res.writeHead(200, { "Content-Type": mime });
  res.end(data);
});

function corsHeaders(contentType) {
  return {
    "Content-Type": contentType,
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET",
  };
}

/** Proxy a remote URL, following redirects, and add CORS headers. */
function proxyFetch(url, res, redirectsLeft = 5) {
  if (redirectsLeft <= 0) {
    res.writeHead(502, corsHeaders("text/plain"));
    res.end("Too many redirects");
    return;
  }

  const doRequest = url.startsWith("https://") ? httpsRequest : httpRequest;

  const proxyReq = doRequest(url, (proxyRes) => {
    // Follow redirects.
    if ([301, 302, 307, 308].includes(proxyRes.statusCode) && proxyRes.headers.location) {
      proxyRes.resume(); // drain
      return proxyFetch(proxyRes.headers.location, res, redirectsLeft - 1);
    }

    const headers = {
      "Access-Control-Allow-Origin": "*",
      "Access-Control-Allow-Methods": "GET",
    };
    if (proxyRes.headers["content-type"]) {
      headers["Content-Type"] = proxyRes.headers["content-type"];
    }
    if (proxyRes.headers["content-length"]) {
      headers["Content-Length"] = proxyRes.headers["content-length"];
    }

    res.writeHead(proxyRes.statusCode, headers);
    proxyRes.pipe(res);
  });

  proxyReq.on("error", (err) => {
    res.writeHead(502, corsHeaders("text/plain"));
    res.end(`Proxy error: ${err.message}`);
  });

  proxyReq.end();
}

server.listen(PORT, "0.0.0.0", () => {
  console.log(`OASIS_OS dev server: http://localhost:${PORT}/www/`);
  console.log(`CORS proxy:          http://localhost:${PORT}/cors-proxy?url=...`);
});
