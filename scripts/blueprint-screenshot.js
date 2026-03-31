#!/usr/bin/env node
// Takes screenshots of blueprint.html from multiple camera angles.
// Usage: node scripts/blueprint-screenshot.js [diagram]
// Starts a local HTTP server, launches headless Chromium, captures screenshots.

const http = require('http');
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer');

const SITE_DIR = path.join(__dirname, '..', 'site');
const OUT_DIR = path.join(__dirname, '..', 'screenshots', 'blueprint');
const DIAGRAM = process.argv[2] || 'psp-usb-adapter';
const WIDTH = 1366;
const HEIGHT = 768;

// Camera presets: [phi, theta, r] — spherical coords
const VIEWS = [
  { name: 'front',      phi: Math.PI/2,   theta: 0,          r: 30,  desc: 'straight-on front' },
  { name: 'hero',       phi: Math.PI/2.8, theta: Math.PI/8,  r: 30,  desc: 'hero angle (default)' },
  { name: 'top-down',   phi: 0.3,         theta: 0,          r: 35,  desc: 'near top-down' },
  { name: 'side-left',  phi: Math.PI/2.5, theta: Math.PI/2,  r: 28,  desc: 'left side' },
  { name: 'close-top',  phi: Math.PI/3,   theta: Math.PI/6,  r: 18,  desc: 'close-up on top edge' },
  { name: 'exploded',   phi: Math.PI/3,   theta: Math.PI/5,  r: 40,  desc: 'exploded view' },
];

// Simple static file server
function startServer() {
  return new Promise((resolve) => {
    const MIME = {
      '.html': 'text/html', '.css': 'text/css', '.js': 'application/javascript',
      '.png': 'image/png', '.jpg': 'image/jpeg', '.svg': 'image/svg+xml'
    };
    const server = http.createServer((req, res) => {
      const urlPath = req.url.split('?')[0];
      let filePath = path.join(SITE_DIR, urlPath === '/' ? 'index.html' : urlPath);
      const ext = path.extname(filePath);
      fs.readFile(filePath, (err, data) => {
        if (err) { res.writeHead(404); res.end('Not found'); return; }
        res.writeHead(200, { 'Content-Type': MIME[ext] || 'application/octet-stream' });
        res.end(data);
      });
    });
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port;
      resolve({ server, port });
    });
  });
}

async function main() {
  fs.mkdirSync(OUT_DIR, { recursive: true });

  const { server, port } = await startServer();
  console.log(`Server on http://127.0.0.1:${port}`);

  const browser = await puppeteer.launch({
    headless: 'new',
    executablePath: '/snap/bin/chromium',
    args: ['--no-sandbox', '--disable-setuid-sandbox',
           '--enable-webgl', '--use-gl=angle', '--use-angle=swiftshader',
           '--enable-unsafe-swiftshader',
           `--window-size=${WIDTH},${HEIGHT}`],
  });

  const page = await browser.newPage();
  await page.setViewport({ width: WIDTH, height: HEIGHT });

  const url = `http://127.0.0.1:${port}/blueprint.html?diagram=${DIAGRAM}`;
  console.log(`Loading ${url}`);
  await page.goto(url, { waitUntil: 'load', timeout: 15000 });
  // Wait for Three.js to render
  await new Promise(r => setTimeout(r, 2000));

  for (const view of VIEWS) {
    console.log(`  Capturing: ${view.name} (${view.desc})`);

    // Set camera via orbit controls and optionally explode slider
    await page.evaluate((v) => {
      if (window.orbit) {
        window.orbit.phi = v.phi;
        window.orbit.theta = v.theta;
        window.orbit.r = v.r;
        window.camUpdate();
      }
    }, { phi: view.phi, theta: view.theta, r: view.r });

    // For exploded view, push the slider
    if (view.name === 'exploded') {
      await page.evaluate(() => {
        const slider = document.getElementById('explode');
        if (slider) {
          slider.value = 80;
          slider.dispatchEvent(new Event('input'));
        }
      });
    } else {
      await page.evaluate(() => {
        const slider = document.getElementById('explode');
        if (slider) {
          slider.value = 20;
          slider.dispatchEvent(new Event('input'));
        }
      });
    }

    await new Promise(r => setTimeout(r, 500)); // let render settle
    const filename = path.join(OUT_DIR, `${DIAGRAM}-${view.name}.png`);
    await page.screenshot({ path: filename });
    console.log(`    -> ${filename}`);
  }

  await browser.close();
  server.close();
  console.log(`\nDone! ${VIEWS.length} screenshots in ${OUT_DIR}`);
}

main().catch(e => { console.error(e); process.exit(1); });
