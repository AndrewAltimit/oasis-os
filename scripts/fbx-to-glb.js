#!/usr/bin/env node
// Converts FBX to a lightweight GLB using puppeteer + Three.js FBXLoader
// Usage: node scripts/fbx-to-glb.js <input.fbx> <output.glb>

const fs = require('fs');
const path = require('path');
const http = require('http');
const puppeteer = require('puppeteer');

const INPUT = process.argv[2] || '/home/mikunpc/Downloads/PSP_Model/psp_low_final.fbx';
const OUTPUT = process.argv[3] || path.join(__dirname, '..', 'site', 'models', 'psp.glb');

const PROJ_ROOT = path.join(__dirname, '..');

async function main() {
  // Serve project root so HTML can load scripts from site/js/
  const server = http.createServer((req, res) => {
    const urlPath = req.url.split('?')[0];
    const filePath = path.join(PROJ_ROOT, urlPath === '/' ? 'index.html' : urlPath);
    fs.readFile(filePath, (err, data) => {
      if (err) { res.writeHead(404); res.end('Not found'); return; }
      res.writeHead(200); res.end(data);
    });
  });

  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const port = server.address().port;

  const browser = await puppeteer.launch({
    headless: 'new',
    executablePath: '/snap/bin/chromium',
    args: ['--no-sandbox', '--disable-setuid-sandbox', '--enable-webgl',
           '--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader']
  });

  const page = await browser.newPage();
  page.on('console', m => console.log('PAGE:', m.text()));
  page.on('pageerror', e => console.log('ERR:', e.message));

  await page.goto(`http://127.0.0.1:${port}/scripts/fbx-to-glb.html`, {
    waitUntil: 'networkidle0', timeout: 15000
  });

  // Read FBX file and pass to browser
  const fbxData = fs.readFileSync(INPUT);
  console.log(`Loading FBX: ${INPUT} (${(fbxData.length/1024).toFixed(0)} KB)`);

  // Transfer buffer to page and convert
  const stats = await page.evaluate(async (fbxBase64) => {
    const binary = atob(fbxBase64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return window.convertFBX(bytes.buffer);
  }, fbxData.toString('base64'));

  console.log('Mesh stats:', JSON.stringify(stats, null, 2));

  // Extract positions and indices arrays
  const exportData = await page.evaluate(() => {
    const d = window._exportData;
    return {
      positions: Array.from(d.positions),
      indices: Array.from(d.indices)
    };
  });

  console.log(`Positions: ${exportData.positions.length/3} vertices`);
  console.log(`Indices: ${exportData.indices.length/3} triangles`);

  // Build a minimal GLB (glTF binary)
  const glb = buildGLB(
    new Float32Array(exportData.positions),
    new Uint32Array(exportData.indices),
    stats.boundingBox
  );

  fs.mkdirSync(path.dirname(OUTPUT), { recursive: true });
  fs.writeFileSync(OUTPUT, glb);
  console.log(`\nWrote: ${OUTPUT} (${(glb.length/1024).toFixed(0)} KB)`);

  await browser.close();
  server.close();
}

// Build minimal GLB with just positions + indices
function buildGLB(positions, indices, bbox) {
  const posBytes = Buffer.from(positions.buffer);
  const idxBytes = Buffer.from(indices.buffer);

  // Pad to 4-byte alignment
  const posPad = (4 - posBytes.length % 4) % 4;
  const idxPad = (4 - idxBytes.length % 4) % 4;

  const binLength = posBytes.length + posPad + idxBytes.length + idxPad;

  const gltf = {
    asset: { version: '2.0', generator: 'oasis-blueprint-converter' },
    scene: 0,
    scenes: [{ nodes: [0] }],
    nodes: [{ mesh: 0, name: 'PSP-3001' }],
    meshes: [{
      primitives: [{
        attributes: { POSITION: 0 },
        indices: 1,
        mode: 4 // TRIANGLES
      }]
    }],
    accessors: [
      {
        bufferView: 0,
        componentType: 5126, // FLOAT
        count: positions.length / 3,
        type: 'VEC3',
        min: bbox.min,
        max: bbox.max
      },
      {
        bufferView: 1,
        componentType: 5125, // UNSIGNED_INT
        count: indices.length,
        type: 'SCALAR'
      }
    ],
    bufferViews: [
      {
        buffer: 0,
        byteOffset: 0,
        byteLength: posBytes.length,
        target: 34962 // ARRAY_BUFFER
      },
      {
        buffer: 0,
        byteOffset: posBytes.length + posPad,
        byteLength: idxBytes.length,
        target: 34963 // ELEMENT_ARRAY_BUFFER
      }
    ],
    buffers: [{ byteLength: binLength }]
  };

  const jsonStr = JSON.stringify(gltf);
  const jsonPad = (4 - jsonStr.length % 4) % 4;
  const jsonBuf = Buffer.from(jsonStr + ' '.repeat(jsonPad));
  const binBuf = Buffer.concat([posBytes, Buffer.alloc(posPad), idxBytes, Buffer.alloc(idxPad)]);

  // GLB header: magic(4) + version(4) + length(4) = 12 bytes
  // JSON chunk: length(4) + type(4) + data
  // BIN chunk: length(4) + type(4) + data
  const totalLength = 12 + 8 + jsonBuf.length + 8 + binBuf.length;

  const out = Buffer.alloc(totalLength);
  let off = 0;

  // Header
  out.writeUInt32LE(0x46546C67, off); off += 4; // glTF magic
  out.writeUInt32LE(2, off); off += 4;           // version
  out.writeUInt32LE(totalLength, off); off += 4;

  // JSON chunk
  out.writeUInt32LE(jsonBuf.length, off); off += 4;
  out.writeUInt32LE(0x4E4F534A, off); off += 4; // JSON type
  jsonBuf.copy(out, off); off += jsonBuf.length;

  // BIN chunk
  out.writeUInt32LE(binBuf.length, off); off += 4;
  out.writeUInt32LE(0x004E4942, off); off += 4; // BIN type
  binBuf.copy(out, off);

  return out;
}

main().catch(e => { console.error(e); process.exit(1); });
