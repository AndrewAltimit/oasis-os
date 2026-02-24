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

  setupTerminal();
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
// Terminal UI
// -----------------------------------------------------------------------

function setupTerminal() {
  const outputEl = document.getElementById("output");
  const cmdEl = document.getElementById("cmd");

  // Print welcome message.
  appendOutput("OASIS_OS [WASM]  —  type 'help' for commands\n");

  cmdEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      const cmd = cmdEl.value.trim();
      if (!cmd) return;

      appendOutput(`oasis> ${cmd}\n`);
      cmdEl.value = "";

      const result = oasis.send_command(cmd);
      if (result) {
        appendOutput(result.endsWith("\n") ? result : result + "\n");
      }
    }
  });

  // Focus the terminal input on page load.
  cmdEl.focus();

  function appendOutput(text) {
    outputEl.textContent += text;
    outputEl.scrollTop = outputEl.scrollHeight;
  }
}

// -----------------------------------------------------------------------
// Start
// -----------------------------------------------------------------------

boot().catch((err) => {
  console.error("OASIS_OS boot failed:", err);
  const outputEl = document.getElementById("output");
  if (outputEl) {
    outputEl.textContent = `Boot error: ${err}\n`;
  }
});
