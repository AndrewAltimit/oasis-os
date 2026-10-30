// OASIS_OS WASM bootstrap
//
// Loads the wasm-pack output and runs the main loop via requestAnimationFrame.

import init, { OasisWasm } from "../pkg/oasis_backend_wasm.js";

let oasis = null;
let lastTime = 0;

// -----------------------------------------------------------------------
// Canvas sizing — replaces CSS object-fit: contain which Firefox does not
// reliably apply to <canvas> elements on desktop.
//
// The canvas keeps its buffer aspect ratio and is letterboxed inside
// #container (which centers it). Scaling is a whole number when the
// container allows 2x or more (crisp pixel art for the 480x272 skins);
// below that it is fractional, rounded *down* to whole device pixels so
// the canvas never overflows the container (overflow: hidden would clip
// its right / bottom edge — the status-bar clock lives there).
// -----------------------------------------------------------------------

function fitScale(bufW, bufH, availW, availH) {
  const fit = Math.min(availW / bufW, availH / bufH);
  return fit >= 2 ? Math.floor(fit) : fit;
}

function fitCanvas() {
  const canvas = document.getElementById("oasis");
  const container = document.getElementById("container");
  if (!canvas || !container) return;

  const bufW = canvas.width;
  const bufH = canvas.height;
  if (bufW === 0 || bufH === 0) return;

  const availW = container.clientWidth;
  const availH = container.clientHeight;
  if (availW === 0 || availH === 0) return;

  const dpr = window.devicePixelRatio || 1;
  const scale = fitScale(bufW, bufH, availW, availH);
  const cssW = Math.floor(bufW * scale * dpr) / dpr;
  const cssH = Math.floor(bufH * scale * dpr) / dpr;
  canvas.style.width = cssW + "px";
  canvas.style.height = cssH + "px";
}

// Re-fit whenever the stage changes size: window resizes, but also the
// demo bar wrapping once its web font loads, or the controls card
// toggling — none of which fire a window "resize".
function watchCanvasFit() {
  window.addEventListener("resize", fitCanvas);
  const container = document.getElementById("container");
  if (container && typeof ResizeObserver === "function") {
    new ResizeObserver(fitCanvas).observe(container);
  }
}

// -----------------------------------------------------------------------
// Boot
// -----------------------------------------------------------------------

async function boot() {
  // Size canvas immediately so it fills the viewport during WASM download
  // instead of appearing at its small intrinsic resolution.
  fitCanvas();
  watchCanvasFit();

  await init();

  const params = new URLSearchParams(window.location.search);
  const skinParam = params.get("skin");
  const appParam = params.get("app");
  oasis = new OasisWasm("oasis", skinParam || undefined);

  // Expose instance globally for automated tests (e.g. Playwright).
  window.__oasis = oasis;
  window.__oasisReady = true;

  // Re-fit in case the skin changed the canvas buffer dimensions.
  fitCanvas();

  // Auto-launch an app if ?app= is specified (e.g. ?app=TV+Guide).
  if (appParam) {
    oasis.launch_app(appParam);
  }

  // Focus canvas for keyboard input.
  const canvas = document.getElementById("oasis");
  if (canvas) canvas.focus();

  requestAnimationFrame(tick);
}

// -----------------------------------------------------------------------
// Main loop
// -----------------------------------------------------------------------

function tick(now) {
  const dt = lastTime ? (now - lastTime) / 1000 : 0;
  lastTime = now;

  oasis.tick(dt);
  requestAnimationFrame(tick);
}

// -----------------------------------------------------------------------
// Start
// -----------------------------------------------------------------------

boot().catch((err) => {
  console.error("OASIS_OS boot failed:", err);
});
