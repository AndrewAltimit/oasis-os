// OASIS_OS WASM bootstrap
//
// Loads the wasm-pack output and runs the main loop via requestAnimationFrame.

import init, { OasisWasm } from "../pkg/oasis_backend_wasm.js";

let oasis = null;
let lastTime = 0;

// -----------------------------------------------------------------------
// Canvas sizing — replaces CSS object-fit: contain which Firefox does not
// reliably apply to <canvas> elements on desktop.
// -----------------------------------------------------------------------

function fitCanvas() {
  const canvas = document.getElementById("oasis");
  const container = document.getElementById("container");
  if (!canvas || !container) return;

  const bufW = canvas.width;
  const bufH = canvas.height;
  if (bufW === 0 || bufH === 0) return;

  const availW = container.clientWidth;
  const availH = container.clientHeight;

  const scale = Math.min(availW / bufW, availH / bufH);
  canvas.style.width = Math.floor(bufW * scale) + "px";
  canvas.style.height = Math.floor(bufH * scale) + "px";
}

// -----------------------------------------------------------------------
// Boot
// -----------------------------------------------------------------------

async function boot() {
  // Size canvas immediately so it fills the viewport during WASM download
  // instead of appearing at its small intrinsic resolution.
  fitCanvas();
  window.addEventListener("resize", fitCanvas);

  await init();

  const skinParam = new URLSearchParams(window.location.search).get("skin");
  oasis = new OasisWasm("oasis", skinParam || undefined);

  // Expose instance globally for automated tests (e.g. Playwright).
  window.__oasis = oasis;
  window.__oasisReady = true;

  // Re-fit in case the skin changed the canvas buffer dimensions.
  fitCanvas();

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
