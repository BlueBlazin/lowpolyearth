#!/usr/bin/env node
// Requires sharp. SVGs remain the editable source; the game embeds the PNGs.
const fs = require('node:fs/promises');
const path = require('node:path');
const sharp = require('sharp');

async function main() {
  const directory = path.join(__dirname, '../crates/vearth-app/assets/ui');
  for (const name of ['search', 'settings', 'x']) {
    const source = (await fs.readFile(path.join(directory, `${name}.svg`), 'utf8'))
      .replace('stroke="currentColor"', 'stroke="#ffffff"')
      .replace('stroke-width="2"', 'stroke-width="1.7"');
    await sharp(Buffer.from(source), { density: 384 })
      .resize(96, 96).ensureAlpha().png()
      .toFile(path.join(directory, `${name}.png`));
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
