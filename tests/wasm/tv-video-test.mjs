#!/usr/bin/env node
// TV Guide video playback diagnostic test.
//
// Launches the TV Guide, tunes a channel by clicking inside the window,
// and verifies the in-canvas video pipeline.
//
// Usage:
//   node tv-video-test.mjs

import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { resolve, extname } from "node:path";
import { chromium, firefox } from "playwright";

// ---------------------------------------------------------------------------
// Server — no COEP so cross-origin video from archive.org is not blocked.
// ---------------------------------------------------------------------------

const MIME = {
  ".html": "text/html",
  ".css": "text/css",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".wasm": "application/wasm",
  ".png": "image/png",
  ".json": "application/json",
};

function startServer(port = 0) {
  const root = resolve(new URL(".", import.meta.url).pathname, "../..");
  const server = createServer(async (req, res) => {
    let url = new URL(req.url, "http://localhost").pathname;
    if (url === "/") url = "/www/index.html";
    const filePath = resolve(root, `.${url}`);
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
    res.writeHead(200, { "Content-Type": mime });
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async function waitForOasis(page, timeout = 15000) {
  await page.waitForFunction("window.__oasisReady === true", null, {
    timeout,
  });
}

async function settle(page, ms) {
  const frames = Math.max(1, Math.floor(ms / 16));
  for (let i = 0; i < frames; i++) {
    await page.evaluate(() => window.__oasis.tick(0.016));
    await page.waitForTimeout(16);
  }
}

function log(msg) {
  console.log(msg);
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

log("=== OASIS TV Guide Video Playback Test ===\n");

const srv = await startServer(0);
log(`Server: ${srv.url}\n`);

// Use Firefox — it includes H.264/AAC codecs out of the box, unlike
// Playwright's bundled Chromium which lacks proprietary codecs.
const browser = await firefox.launch({ headless: true });
let exitCode = 0;

try {
  const context = await browser.newContext({
    viewport: { width: 1280, height: 720 },
  });
  const page = await context.newPage();

  const consoleLogs = [];
  const pageErrors = [];
  const networkFailures = [];

  page.on("console", (msg) =>
    consoleLogs.push({ type: msg.type(), text: msg.text() }),
  );
  page.on("pageerror", (err) => pageErrors.push(err.message));
  page.on("requestfailed", (req) =>
    networkFailures.push({
      url: req.url(),
      failure: req.failure()?.errorText || "unknown",
    }),
  );

  // -----------------------------------------------------------------------
  // Step 1: Boot OASIS with TV Guide
  // -----------------------------------------------------------------------
  log("[1] Booting OASIS with TV Guide ...");
  await page.goto(`${srv.url}/www/index.html?skin=classic&app=TV+Guide`);
  await waitForOasis(page);
  await settle(page, 1500);
  log("    OASIS ready.\n");

  // -----------------------------------------------------------------------
  // Step 2: Wait for catalogs to load from archive.org
  // -----------------------------------------------------------------------
  log("[2] Waiting for channel catalogs to load (8s) ...");
  await settle(page, 8000);

  // Snapshot: how many catalogs loaded?
  const catalogInfo = await page.evaluate(() => {
    const o = window.__oasis;
    for (let i = 0; i < 5; i++) o.tick(0.016);
    // We can't directly access guide state from JS, but we can
    // check network activity indirectly via console output.
    return { ready: true };
  });
  log("    Catalogs should be loaded by now.\n");

  // -----------------------------------------------------------------------
  // Step 3: Take pre-tune screenshot to see the guide state
  // -----------------------------------------------------------------------
  log("[3] Pre-tune screenshot ...");
  await settle(page, 200);
  const prePath = new URL("./tv-pretune.png", import.meta.url).pathname;
  await page.screenshot({ path: prePath });
  log(`    Saved: ${prePath}\n`);

  // -----------------------------------------------------------------------
  // Step 4: Click inside the TV Guide window to focus it
  // -----------------------------------------------------------------------
  log("[4] Clicking inside TV Guide window to focus ...");
  // The TV Guide window occupies the full client area on classic skin.
  // Canvas is 480x272, displayed at larger CSS size.
  // We need to click in canvas coordinates, which Playwright maps via CSS.
  // The canvas element has CSS dimensions set by fitCanvas().
  // Playwright clicks are in CSS pixels. We need to click at a location
  // that maps to the TV Guide content area.
  //
  // The channel grid starts around y=100 in canvas coords.
  // Let's click near the middle of the canvas, which should be in the
  // channel grid area.
  const canvasBox = await page.locator("#oasis").boundingBox();
  if (!canvasBox) throw new Error("Canvas not found");

  const canvasSize = await page.evaluate(() => ({
    w: window.__oasis.screen_width(),
    h: window.__oasis.screen_height(),
  }));

  // Compute CSS-to-canvas scale.
  const scaleX = canvasBox.width / canvasSize.w;
  const scaleY = canvasBox.height / canvasSize.h;

  // Click on channel row 2 (second channel, around y=160 in canvas coords,
  // x=center). This should select a channel and focus the window.
  const clickCanvasX = canvasSize.w / 2;
  const clickCanvasY = 160; // second channel row in the grid
  const clickCssX = canvasBox.x + clickCanvasX * scaleX;
  const clickCssY = canvasBox.y + clickCanvasY * scaleY;

  log(`    Canvas: ${canvasSize.w}x${canvasSize.h}, CSS box: ${canvasBox.width.toFixed(0)}x${canvasBox.height.toFixed(0)}`);
  log(`    Clicking at canvas(${clickCanvasX}, ${clickCanvasY}) = CSS(${clickCssX.toFixed(0)}, ${clickCssY.toFixed(0)})`);
  await page.mouse.click(clickCssX, clickCssY);
  await settle(page, 300);
  log("    Clicked.\n");

  // -----------------------------------------------------------------------
  // Step 5: Navigate down to find a channel with content and press Enter
  // -----------------------------------------------------------------------
  log("[5] Navigating channels and attempting tune ...");

  let tuned = false;
  for (let attempt = 0; attempt < 6; attempt++) {
    // Press Enter to try tuning current channel.
    await page.keyboard.press("Enter");
    await settle(page, 800);

    const videoCount = await page.evaluate(
      () => document.querySelectorAll("video").length,
    );
    if (videoCount > 0) {
      log(`    Video element created on attempt ${attempt + 1}!`);
      tuned = true;
      break;
    }

    // No video — this channel might have no content. Press Down for next.
    await page.keyboard.press("ArrowDown");
    await settle(page, 400);
  }

  if (!tuned) {
    log("    No channel produced a video element after 6 attempts.");
    log("    Trying to click further down in the grid ...\n");

    // Try clicking on rows further down (y=200, 230, 260).
    for (const rowY of [200, 230, 260]) {
      const cssX2 = canvasBox.x + (canvasSize.w / 2) * scaleX;
      const cssY2 = canvasBox.y + rowY * scaleY;
      await page.mouse.click(cssX2, cssY2);
      await settle(page, 300);
      await page.keyboard.press("Enter");
      await settle(page, 800);

      const vc = await page.evaluate(
        () => document.querySelectorAll("video").length,
      );
      if (vc > 0) {
        log(`    Video created after clicking row at y=${rowY}!`);
        tuned = true;
        break;
      }
    }
  }

  log(tuned ? "    Tune succeeded.\n" : "    Tune did not produce a <video> element.\n");

  // -----------------------------------------------------------------------
  // Step 6: Video element diagnostics
  // -----------------------------------------------------------------------
  log("[6] Video element state ...");
  await settle(page, 2000);

  const videoState = await page.evaluate(() => {
    const videos = document.querySelectorAll("video");
    return {
      count: videos.length,
      details: Array.from(videos).map((v) => ({
        src: v.src,
        crossOrigin: v.crossOrigin,
        muted: v.muted,
        paused: v.paused,
        readyState: v.readyState,
        networkState: v.networkState,
        currentTime: v.currentTime,
        duration: v.duration,
        videoWidth: v.videoWidth,
        videoHeight: v.videoHeight,
        error: v.error
          ? { code: v.error.code, message: v.error.message }
          : null,
        display: v.style.display,
      })),
    };
  });

  log(`    Count: ${videoState.count}`);
  for (const v of videoState.details) {
    log(`    src: ${v.src.slice(0, 120)}${v.src.length > 120 ? "..." : ""}`);
    log(`    crossOrigin=${v.crossOrigin} muted=${v.muted} paused=${v.paused}`);
    log(`    readyState=${v.readyState} networkState=${v.networkState}`);
    log(`    time=${v.currentTime?.toFixed(2)}/${v.duration} size=${v.videoWidth}x${v.videoHeight}`);
    log(`    error=${JSON.stringify(v.error)} display=${v.display}`);
  }
  log("");

  // Wait more for buffering if video exists but not ready.
  if (videoState.count > 0 && videoState.details[0].readyState < 2) {
    log("[6b] Video not ready, waiting 10s more for buffering ...");
    await settle(page, 10000);
    const v2 = await page.evaluate(() => {
      const v = document.querySelector("video");
      if (!v) return null;
      return {
        readyState: v.readyState,
        networkState: v.networkState,
        currentTime: v.currentTime,
        paused: v.paused,
        error: v.error
          ? { code: v.error.code, message: v.error.message }
          : null,
      };
    });
    if (v2) {
      log(`    readyState=${v2.readyState} networkState=${v2.networkState}`);
      log(`    time=${v2.currentTime?.toFixed(2)} paused=${v2.paused} error=${JSON.stringify(v2.error)}`);
    }
    log("");
  }

  // -----------------------------------------------------------------------
  // Step 7: Capture canvas check
  // -----------------------------------------------------------------------
  log("[7] Offscreen canvases ...");
  const canvases = await page.evaluate(() => {
    const all = document.querySelectorAll("canvas");
    const main = document.getElementById("oasis");
    const off = Array.from(all).filter((c) => c !== main);
    return {
      total: all.length,
      offscreen: off.length,
      details: off.map((c) => ({ w: c.width, h: c.height })),
    };
  });
  log(`    Total: ${canvases.total}, Offscreen: ${canvases.offscreen}`);
  for (const c of canvases.details) log(`    - ${c.w}x${c.h}`);
  log("");

  // -----------------------------------------------------------------------
  // Step 8: Final screenshot
  // -----------------------------------------------------------------------
  log("[8] Post-tune screenshot ...");
  const postPath = new URL("./tv-posttune.png", import.meta.url).pathname;
  await page.screenshot({ path: postPath });
  log(`    Saved: ${postPath}\n`);

  // -----------------------------------------------------------------------
  // Diagnostics dump
  // -----------------------------------------------------------------------
  log("=== Diagnostics ===\n");

  if (pageErrors.length > 0) {
    log("Page errors:");
    for (const e of pageErrors) log(`  ${e}`);
    log("");
  }
  if (networkFailures.length > 0) {
    log(`Network failures (${networkFailures.length}):`);
    for (const f of networkFailures.slice(0, 10)) {
      log(`  ${f.failure}: ${f.url.slice(0, 150)}`);
    }
    if (networkFailures.length > 10)
      log(`  ... and ${networkFailures.length - 10} more`);
    log("");
  }

  const relevant = consoleLogs.filter(
    (l) =>
      l.type === "error" ||
      (l.type === "warning" &&
        !l.text.includes("sandbox")) ||
      /video|tune|cors|media|autoplay|blocked|texture/i.test(l.text),
  );
  if (relevant.length > 0) {
    log("Relevant console:");
    for (const l of relevant) log(`  [${l.type}] ${l.text.slice(0, 250)}`);
    log("");
  }

  // Show ALL console logs for full debugging.
  log("All console messages:");
  for (const l of consoleLogs) {
    log(`  [${l.type}] ${l.text.slice(0, 250)}`);
  }
  log("");

  // -----------------------------------------------------------------------
  // Verdict
  // -----------------------------------------------------------------------
  log("=== Verdict ===\n");
  const hasVideo = videoState.count > 0;
  const videoErr = videoState.details.find((v) => v.error)?.error;
  const playing = videoState.details.some(
    (v) => !v.paused && v.readyState >= 2,
  );
  const hasCap = canvases.offscreen > 0;

  log(`  Video created:   ${hasVideo}`);
  log(`  Video error:     ${videoErr ? JSON.stringify(videoErr) : "none"}`);
  log(`  Video playing:   ${playing}`);
  log(`  Capture canvas:  ${hasCap}`);

  if (!hasVideo) {
    log("\n  FAIL: No <video> element. Tune never reached VideoPlayer.start().");
    exitCode = 1;
  } else if (videoErr) {
    log(`\n  FAIL: Video error ${videoErr.code}.`);
    exitCode = 1;
  } else if (!playing) {
    log("\n  WARN: Video exists but not yet playing (may need more buffer time).");
    // Not necessarily a failure — could be slow network.
  } else {
    log("\n  Video pipeline is working!");
  }

  await context.close();
} catch (err) {
  console.error(`\nCrashed: ${err.message}\n${err.stack}`);
  exitCode = 1;
} finally {
  await browser.close();
  srv.close();
}

log("");
process.exit(exitCode);
