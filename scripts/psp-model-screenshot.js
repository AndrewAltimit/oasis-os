#!/usr/bin/env node
// Screenshots the PSP model comparison grid
const http = require('http');
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer');

const PROJ = path.join(__dirname, '..');
const testPage = process.argv[2] || 'psp-model-compare';
const OUT = path.join(PROJ, 'screenshots', 'blueprint', testPage + '.png');

async function main(){
  const server = http.createServer((req, res) => {
    const urlPath = req.url.split('?')[0];
    // Serve from both project root (for scripts/) and site/ (for models/)
    let filePath = path.join(PROJ, urlPath === '/' ? 'index.html' : urlPath);
    // Also try site/ prefix for model files
    if (!fs.existsSync(filePath) && urlPath.startsWith('/models/')) {
      filePath = path.join(PROJ, 'site', urlPath);
    }
    fs.readFile(filePath, (err, data) => {
      if (err) { console.log('404:', urlPath); res.writeHead(404); res.end('Not found'); return; }
      const ext = path.extname(filePath);
      const mime = {'.html':'text/html','.js':'application/javascript','.glb':'model/gltf-binary'}[ext] || 'application/octet-stream';
      res.writeHead(200, {'Content-Type': mime});
      res.end(data);
    });
  });

  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const port = server.address().port;
  console.log('Server on port', port);

  const browser = await puppeteer.launch({
    headless: 'new',
    executablePath: '/snap/bin/chromium',
    args: ['--no-sandbox','--disable-setuid-sandbox','--enable-webgl',
           '--use-gl=angle','--use-angle=swiftshader','--enable-unsafe-swiftshader',
           '--window-size=1920,1080']
  });

  const page = await browser.newPage();
  page.on('console', m => console.log('PAGE:', m.text()));
  page.on('pageerror', e => console.log('ERR:', e.message));
  await page.setViewport({width:1920, height:1080});
  const testPage = process.argv[2] || 'psp-model-compare';
  await page.goto(`http://127.0.0.1:${port}/scripts/${testPage}.html`, {
    waitUntil: 'networkidle0', timeout: 20000
  });

  // Wait for model to load and render
  await new Promise(r => setTimeout(r, 4000));
  await page.screenshot({path: OUT});
  console.log('Wrote:', OUT);

  await browser.close();
  server.close();
}

main().catch(e => { console.error(e); process.exit(1); });
