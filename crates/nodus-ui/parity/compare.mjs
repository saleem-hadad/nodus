// usage: node compare.mjs <golden dir> [scene.theme [x y w h]]
// With a scene: writes <golden>/../rust-diffs/<scene>.png (golden | rust |
// pixelmatch diff, side by side, cropped to the logical rectangle) and prints
// how much differs and where. Without: one line per *.rust.png in the folder.
// Needs the parity harness's node_modules (pngjs, pixelmatch): run it with
// NODE_PATH=<this folder>/harness/node_modules (absolute), or from there.
import fs from 'node:fs'
import path from 'node:path'
import { createRequire } from 'node:module'
const require = createRequire(path.join(process.env.NODE_PATH ?? process.cwd(), 'x.js'))
const { PNG } = require('pngjs')
const pixelmatch = (await import(require.resolve('pixelmatch'))).default
const [golden, name, ...crop] = process.argv.slice(2)

const load = (file) => PNG.sync.read(fs.readFileSync(path.join(golden, file)))

function compare(name, crop) {
  const a = load(`${name}.png`)
  const b = load(`${name}.rust.png`)
  let [x, y, w, h] = crop.length ? crop.map((v) => Math.round(Number(v) * 2)) : [0, 0, a.width, a.height]
  w = Math.min(w, a.width - x, b.width - x)
  h = Math.min(h, a.height - y, b.height - y)
  const cut = (img) => {
    const out = new PNG({ width: w, height: h })
    for (let r = 0; r < h; r++) img.data.copy(out.data, r * w * 4, ((y + r) * img.width + x) * 4, ((y + r) * img.width + x + w) * 4)
    return out
  }
  const ca = cut(a)
  const cb = cut(b)
  const diff = new PNG({ width: w, height: h })
  const n = pixelmatch(ca.data, cb.data, diff.data, w, h, { threshold: 0.1 })
  return { n, w, h, ca, cb, diff }
}

if (!name) {
  for (const file of fs.readdirSync(golden).filter((f) => f.endsWith('.rust.png')).sort()) {
    const scene = file.replace('.rust.png', '')
    if (!fs.existsSync(path.join(golden, `${scene}.png`))) continue
    const { n, w, h } = compare(scene, [])
    console.log(`${scene.padEnd(36)} ${((100 * n) / (w * h)).toFixed(2)}%`)
  }
} else {
  const { n, w, h, ca, cb, diff } = compare(name, crop)
  const side = new PNG({ width: w * 3, height: h })
  for (let r = 0; r < h; r++) {
    ca.data.copy(side.data, r * w * 3 * 4, r * w * 4, (r + 1) * w * 4)
    cb.data.copy(side.data, (r * w * 3 + w) * 4, r * w * 4, (r + 1) * w * 4)
    diff.data.copy(side.data, (r * w * 3 + 2 * w) * 4, r * w * 4, (r + 1) * w * 4)
  }
  const dir = path.join(golden, '..', 'rust-diffs')
  fs.mkdirSync(dir, { recursive: true })
  const out = path.join(dir, `${name}${crop.length ? '-' + crop.join('_') : ''}.png`)
  fs.writeFileSync(out, PNG.sync.write(side))
  console.log(`${name}: ${((100 * n) / (w * h)).toFixed(2)}% differ -> ${out}`)
}
