// usage: node compare.mjs <dir> [x y w h]
// chromium.png against gpui.png in <dir>, case by case (cases.json and
// page.json): differing pixels (pixelmatch), the ink ratio and the shift of
// the ink's centre in device pixels. With a rectangle (logical px), writes
// <dir>/side.png: Chromium | GPUI | diff. Needs pngjs and pixelmatch, as
// ../compare.mjs does (NODE_PATH=<migration/parity>/node_modules).
import fs from 'node:fs'
import path from 'node:path'
import { createRequire } from 'node:module'
const require = createRequire(path.join(process.env.NODE_PATH ?? process.cwd(), 'x.js'))
const { PNG } = require('pngjs')
const pixelmatch = (await import(require.resolve('pixelmatch'))).default
const [dir, ...crop] = process.argv.slice(2)
const A = PNG.sync.read(fs.readFileSync(path.join(dir, 'chromium.png')))
const B = PNG.sync.read(fs.readFileSync(path.join(dir, 'gpui.png')))
const cut = (img, x, y, w, h) => {
  const out = new PNG({ width: w, height: h })
  for (let r = 0; r < h; r++) img.data.copy(out.data, r * w * 4, ((y + r) * img.width + x) * 4, ((y + r) * img.width + x + w) * 4)
  return out
}
function region(x, y, w, h) {
  ;[x, y, w, h] = [x, y, w, h].map((v) => Math.round(v * 2))
  const a = cut(A, x, y, w, h)
  const b = cut(B, x, y, w, h)
  const diff = new PNG({ width: w, height: h })
  const n = pixelmatch(a.data, b.data, diff.data, w, h, { threshold: 0.1 })
  let inkA = 0, inkB = 0, yA = 0, yB = 0, xA = 0, xB = 0
  for (let r = 0; r < h; r++) {
    for (let k = 0; k < w; k++) {
      const i = (r * w + k) * 4
      const va = 255 - a.data[i + 1]
      const vb = 255 - b.data[i + 1]
      inkA += va; inkB += vb; yA += va * r; yB += vb * r; xA += va * k; xB += vb * k
    }
  }
  const shift = (sb, sa) => (inkA && inkB ? sb / inkB - sa / inkA : 0)
  return { n, w, h, a, b, diff, ink: inkA ? inkB / inkA : 1, dx: shift(xB, xA), dy: shift(yB, yA) }
}
if (crop.length) {
  const r = region(...crop.map(Number))
  const side = new PNG({ width: r.w * 3, height: r.h })
  for (let row = 0; row < r.h; row++) {
    r.a.data.copy(side.data, row * r.w * 3 * 4, row * r.w * 4, (row + 1) * r.w * 4)
    r.b.data.copy(side.data, (row * r.w * 3 + r.w) * 4, row * r.w * 4, (row + 1) * r.w * 4)
    r.diff.data.copy(side.data, (row * r.w * 3 + 2 * r.w) * 4, row * r.w * 4, (row + 1) * r.w * 4)
  }
  fs.writeFileSync(path.join(dir, 'side.png'), PNG.sync.write(side))
  console.log(`${r.n} px differ, ink ${r.ink.toFixed(3)}, centre moved (${r.dx.toFixed(2)}, ${r.dy.toFixed(2)}) device px`)
} else {
  const cases = JSON.parse(fs.readFileSync(path.join(dir, 'cases.json')))
  const rects = JSON.parse(fs.readFileSync(path.join(dir, 'page.json')))
  let total = 0
  for (const c of cases) {
    const [, height] = rects[c.id]
    const r = region(c.x - 2, c.y - 2, (c.w ?? 50) + 4, height + 4 + (c.bar ? 3 : 0))
    total += r.n
    const label = `${c.kind} ${c.size}px${c.w ? ` w${c.w}` : ''} ${c.align}${c.clamp ? ` clamp ${c.clamp}` : ''}`
    console.log(`${String(c.id).padStart(2)} ${label.padEnd(34)} ${String(r.n).padStart(5)} px  ink ${r.ink.toFixed(3)}  (${r.dx.toFixed(2)}, ${r.dy.toFixed(2)})  ${c.text.slice(0, 28)}`)
  }
  console.log(`${total} px differ in all`)
}
