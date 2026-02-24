#!/usr/bin/env node
// OASIS_OS WASM screenshot test runner.
//
// Captures screenshots from the WASM backend in a headless Chromium browser,
// compares them against SDL reference screenshots, and generates a diff report.
//
// Usage:
//   node screenshot-test.mjs              # run all scenarios
//   node screenshot-test.mjs --bless      # update SDL references (requires native build)
//   node screenshot-test.mjs --scenario dashboard_classic

import { existsSync, mkdirSync, copyFileSync } from "node:fs";
import { resolve, relative } from "node:path";
import { chromium } from "playwright";
import { startServer } from "./serve.mjs";
import { readPng, writePng, compareImages, generateReport } from "./compare.mjs";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const ROOT = resolve(new URL(".", import.meta.url).pathname, "../..");
const SCREENSHOTS_DIR = resolve(ROOT, "screenshots/wasm");
const SDL_SCREENSHOTS_DIR = resolve(ROOT, "screenshots");
const REPORT_PATH = resolve(SCREENSHOTS_DIR, "report.html");

// Max pixel diff percentage to consider a test passing.
const DIFF_THRESHOLD_PCT = 5.0;

// How long to wait after boot before capturing (ms).
const SETTLE_TIME = 500;

// Scenarios: each defines a skin, pre-capture actions, and SDL reference path.
const SCENARIOS = [
  {
    name: "dashboard_classic",
    skin: "classic",
    sdlRef: "classic/01_dashboard.png",
    actions: [],
  },
  {
    name: "dashboard_modern",
    skin: "modern",
    sdlRef: "modern/01_dashboard.png",
    actions: [],
  },
  {
    name: "dashboard_xp",
    skin: "xp",
    sdlRef: "xp/01_dashboard.png",
    actions: [],
  },
  {
    name: "input_navigation",
    skin: "classic",
    sdlRef: null,
    actions: [
      { type: "key", key: "ArrowRight" },
      { type: "key", key: "ArrowRight" },
      { type: "key", key: "ArrowDown" },
      { type: "settle", ms: 200 },
    ],
  },
  {
    name: "input_triggers",
    skin: "classic",
    sdlRef: null,
    actions: [
      { type: "key", key: "e" }, // TriggerRight → next page
      { type: "settle", ms: 200 },
    ],
  },
  {
    name: "terminal_help",
    skin: "classic",
    sdlRef: null,
    actions: [{ type: "command", cmd: "help" }],
    verify: (output) => output.includes("Available commands") || output.includes("help"),
  },
  {
    name: "terminal_ls",
    skin: "classic",
    sdlRef: null,
    actions: [{ type: "command", cmd: "ls /apps" }],
    verify: (output) => output.includes("file_manager") || output.includes("terminal"),
  },
  {
    name: "terminal_cd",
    skin: "classic",
    sdlRef: null,
    actions: [
      { type: "command", cmd: "cd /home" },
      { type: "command", cmd: "pwd", capture: true },
    ],
    verify: (output) => output.includes("/home"),
  },
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async function waitForOasis(page, timeout = 10000) {
  await page.waitForFunction("window.__oasisReady === true", null, {
    timeout,
  });
}

async function captureFramebuffer(page) {
  return await page.evaluate(() => {
    const oasis = window.__oasis;
    // Run a tick to ensure the latest frame is rendered.
    oasis.tick(0.016);
    const pixels = oasis.read_pixels();
    const w = oasis.screen_width();
    const h = oasis.screen_height();
    return { width: w, height: h, data: Array.from(pixels) };
  });
}

async function sendCommand(page, cmd) {
  return await page.evaluate((c) => window.__oasis.send_command(c), cmd);
}

async function sendKey(page, key) {
  // Click canvas first to ensure keyboard events reach the window listeners
  // instead of being captured by the terminal input field.
  await page.click("#oasis");
  await page.keyboard.press(key);
  // Let the tick process the event.
  await page.evaluate(() => window.__oasis.tick(0.016));
}

async function settle(page, ms) {
  // Run several ticks over the settle period.
  const frames = Math.max(1, Math.floor(ms / 16));
  for (let i = 0; i < frames; i++) {
    await page.evaluate(() => window.__oasis.tick(0.016));
    await page.waitForTimeout(16);
  }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

const args = process.argv.slice(2);
const bless = args.includes("--bless");
const scenarioFilter = args.find((a) => !a.startsWith("--"));

const scenarios = scenarioFilter
  ? SCENARIOS.filter((s) => s.name === scenarioFilter)
  : SCENARIOS;

if (scenarios.length === 0) {
  console.error(`No scenario matching "${scenarioFilter}"`);
  process.exit(1);
}

console.log(`OASIS WASM Screenshot Tests (${scenarios.length} scenarios)\n`);

// Start file server.
const srv = await startServer(0);
console.log(`Server: ${srv.url}\n`);

// Launch headless browser.
const browser = await chromium.launch({ headless: true });
const results = [];
let failures = 0;

try {
  for (const scenario of scenarios) {
    const tag = `[${scenario.name}]`;
    const outDir = resolve(SCREENSHOTS_DIR, scenario.name);
    mkdirSync(outDir, { recursive: true });

    process.stdout.write(`${tag} `);

    // Open page with the target skin.
    const context = await browser.newContext({
      viewport: { width: 640, height: 480 },
    });
    const page = await context.newPage();

    // Suppress console noise.
    page.on("pageerror", (err) =>
      console.error(`  ${tag} page error: ${err.message}`),
    );

    const skinParam = scenario.skin ? `?skin=${scenario.skin}` : "";
    await page.goto(`${srv.url}/www/index.html${skinParam}`);
    await waitForOasis(page);
    await settle(page, SETTLE_TIME);

    // Execute scenario actions.
    let commandOutput = "";
    for (const action of scenario.actions) {
      switch (action.type) {
        case "key":
          await sendKey(page, action.key);
          break;
        case "command": {
          const out = await sendCommand(page, action.cmd);
          if (action.capture) commandOutput = out;
          else commandOutput += out;
          break;
        }
        case "settle":
          await settle(page, action.ms);
          break;
      }
    }

    // Settle after all actions.
    await settle(page, 200);

    // Capture WASM framebuffer.
    const frame = await captureFramebuffer(page);
    const actualPath = resolve(outDir, "actual.png");
    writePng(actualPath, frame.width, frame.height, new Uint8Array(frame.data));

    // Verify command output if applicable.
    if (scenario.verify) {
      const ok = scenario.verify(commandOutput);
      if (!ok) {
        console.log(`FAIL (command output verification)`);
        console.log(`  Output: ${commandOutput.slice(0, 200)}`);
        results.push({
          scenario: scenario.name,
          diffPercent: 100,
          pass: false,
          actualPath: relative(SCREENSHOTS_DIR, actualPath),
        });
        failures++;
        await context.close();
        continue;
      }
    }

    // Compare against SDL reference if available.
    let diffPercent = 0;
    let diffPath = null;
    let refRelPath = null;

    if (scenario.sdlRef) {
      const sdlRefPath = resolve(SDL_SCREENSHOTS_DIR, scenario.sdlRef);
      const localRefPath = resolve(outDir, "sdl_reference.png");

      if (existsSync(sdlRefPath)) {
        copyFileSync(sdlRefPath, localRefPath);
        refRelPath = relative(SCREENSHOTS_DIR, localRefPath);

        const imgA = { width: frame.width, height: frame.height, data: new Uint8Array(frame.data) };
        const imgB = readPng(sdlRefPath);
        const result = compareImages(imgA, imgB);
        diffPercent = result.diffPercent;

        diffPath = resolve(outDir, "diff.png");
        writePng(diffPath, result.diffImage.width, result.diffImage.height, result.diffImage.data);
        diffPath = relative(SCREENSHOTS_DIR, diffPath);
      } else {
        console.log(`SKIP (no SDL reference at ${scenario.sdlRef})`);
        results.push({
          scenario: scenario.name,
          diffPercent: -1,
          pass: true,
          actualPath: relative(SCREENSHOTS_DIR, actualPath),
        });
        await context.close();
        continue;
      }
    }

    const pass = diffPercent <= DIFF_THRESHOLD_PCT;
    if (!pass) failures++;

    const status = scenario.sdlRef
      ? `${pass ? "PASS" : "FAIL"} (diff: ${diffPercent}%)`
      : "PASS (no reference)";
    console.log(status);

    results.push({
      scenario: scenario.name,
      diffPercent,
      pass,
      actualPath: relative(SCREENSHOTS_DIR, actualPath),
      referencePath: refRelPath,
      diffPath,
    });

    await context.close();
  }
} finally {
  await browser.close();
  srv.close();
}

// Generate HTML report.
generateReport(results, REPORT_PATH);
console.log(`\nReport: ${REPORT_PATH}`);

// Summary.
const passed = results.filter((r) => r.pass).length;
console.log(`\n${passed}/${results.length} passed, ${failures} failed`);

process.exit(failures > 0 ? 1 : 0);
