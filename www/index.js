// OASIS_OS WASM bootstrap
//
// Loads the wasm-pack output and runs the main loop via requestAnimationFrame.

import init, { OasisWasm } from "../pkg/oasis_backend_wasm.js";

let oasis = null;
let lastTime = 0;

// -----------------------------------------------------------------------
// Boot
// -----------------------------------------------------------------------

async function boot() {
  await init();

  const skinParam = new URLSearchParams(window.location.search).get("skin");
  oasis = new OasisWasm("oasis", skinParam || undefined);

  // Expose instance globally for automated tests (e.g. Playwright).
  window.__oasis = oasis;
  window.__oasisReady = true;

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
