// PNG image comparison utilities for screenshot tests.
//
// Computes per-pixel diff between two RGBA images and generates a diff
// image with red-highlighted differences.

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { PNG } from "pngjs";

/**
 * Read a PNG file and return { width, height, data: Uint8Array(RGBA) }.
 */
export function readPng(path) {
  const buf = readFileSync(path);
  const png = PNG.sync.read(buf);
  return { width: png.width, height: png.height, data: png.data };
}

/**
 * Write RGBA pixel data to a PNG file.
 */
export function writePng(path, width, height, data) {
  mkdirSync(dirname(path), { recursive: true });
  const png = new PNG({ width, height });
  png.data = Buffer.from(data);
  const buf = PNG.sync.write(png);
  writeFileSync(path, buf);
}

/**
 * Compare two RGBA images pixel-by-pixel.
 *
 * Returns { diffCount, totalPixels, diffPercent, diffImage }.
 * `diffImage` is RGBA data of the same size with red highlighting.
 *
 * `threshold` is the per-channel tolerance (0-255).
 */
export function compareImages(imgA, imgB, threshold = 2) {
  const w = Math.min(imgA.width, imgB.width);
  const h = Math.min(imgA.height, imgB.height);
  const totalPixels = w * h;
  const diffData = new Uint8Array(w * h * 4);
  let diffCount = 0;

  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const idxA = (y * imgA.width + x) * 4;
      const idxB = (y * imgB.width + x) * 4;
      const idxD = (y * w + x) * 4;

      const dr = Math.abs(imgA.data[idxA] - imgB.data[idxB]);
      const dg = Math.abs(imgA.data[idxA + 1] - imgB.data[idxB + 1]);
      const db = Math.abs(imgA.data[idxA + 2] - imgB.data[idxB + 2]);

      if (dr > threshold || dg > threshold || db > threshold) {
        // Red highlight on diff pixels.
        diffData[idxD] = 255;
        diffData[idxD + 1] = 0;
        diffData[idxD + 2] = 0;
        diffData[idxD + 3] = 255;
        diffCount++;
      } else {
        // Dimmed version of original for context.
        diffData[idxD] = imgA.data[idxA] >> 1;
        diffData[idxD + 1] = imgA.data[idxA + 1] >> 1;
        diffData[idxD + 2] = imgA.data[idxA + 2] >> 1;
        diffData[idxD + 3] = 255;
      }
    }
  }

  // If sizes differ, mark extra pixels as diff.
  const maxW = Math.max(imgA.width, imgB.width);
  const maxH = Math.max(imgA.height, imgB.height);
  if (maxW > w || maxH > h) {
    diffCount += maxW * maxH - totalPixels;
  }

  const diffPercent = totalPixels > 0 ? (diffCount / totalPixels) * 100 : 100;

  return {
    diffCount,
    totalPixels,
    diffPercent: Math.round(diffPercent * 100) / 100,
    diffImage: { width: w, height: h, data: diffData },
  };
}

/**
 * Generate a simple HTML report comparing screenshots.
 */
export function generateReport(results, outputPath) {
  mkdirSync(dirname(outputPath), { recursive: true });
  const rows = results
    .map(
      (r) => `
    <tr>
      <td><strong>${r.scenario}</strong></td>
      <td>${r.diffPercent}%</td>
      <td>${r.pass ? "PASS" : "FAIL"}</td>
      <td>
        <div style="display:flex;gap:8px;flex-wrap:wrap">
          ${r.actualPath ? `<div><div style="font-size:11px">WASM</div><img src="${r.actualPath}" width="240"></div>` : ""}
          ${r.referencePath ? `<div><div style="font-size:11px">SDL Reference</div><img src="${r.referencePath}" width="240"></div>` : ""}
          ${r.diffPath ? `<div><div style="font-size:11px">Diff</div><img src="${r.diffPath}" width="240"></div>` : ""}
        </div>
      </td>
    </tr>`,
    )
    .join("\n");

  const html = `<!DOCTYPE html>
<html><head>
<meta charset="utf-8">
<title>OASIS WASM Screenshot Report</title>
<style>
  body { font-family: monospace; background: #111; color: #ccc; padding: 20px; }
  table { border-collapse: collapse; width: 100%; }
  th, td { border: 1px solid #333; padding: 8px; text-align: left; vertical-align: top; }
  th { background: #222; }
  img { image-rendering: pixelated; border: 1px solid #444; }
</style>
</head><body>
<h1>OASIS WASM Screenshot Test Report</h1>
<table>
  <tr><th>Scenario</th><th>Diff %</th><th>Status</th><th>Images</th></tr>
  ${rows}
</table>
</body></html>`;

  writeFileSync(outputPath, html);
}
