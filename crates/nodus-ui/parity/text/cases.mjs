// usage: node cases.mjs <dir>
// Writes the same text cases twice into <dir>: page.html for Chromium
// (chromium.cjs) and cases.json for GPUI (examples/text_parity.rs). Black
// system-font text on white, each case an absolutely positioned div: line
// boxes and baselines at sizes and line heights the app uses, wrapping
// (hanging spaces, overflow-wrap, word-break, alignment), -webkit-line-clamp
// with its ellipsis, and one-line text-overflow: ellipsis.
import fs from 'node:fs'
import path from 'node:path'
const out = process.argv[2]
const cases = []
const add = (c) => cases.push({ id: cases.length, weight: 400, align: 'left', lh: null, ...c })
// 1. Stacks of one-line blocks with a 2px bar under each: line boxes and baselines.
const stacks = [[13, 18.2], [12, 16.8], [13, 16.9], [11, 15.4], [15, 21], [20, 28], [26, 36.4], [12.5, 17.5]]
stacks.forEach(([size, lh], i) => {
  for (let k = 0; k < 6; k++) add({ kind: 'stack', x: 10 + i * 60, y: 10 + k * (lh + 2), w: 50, size, lh, text: 'Hxg', bar: true })
})
// 2. Wrapping: hanging spaces, long words, break-all, centred and right-aligned rows.
const para = 'A panel floats up over what you’re doing. Type or say what you want, and Claude works out whether it’s a new session, a note, a view or a reply to a session.'
let y = 10
for (const [w, align] of [[300, 'left'], [301.5, 'left'], [262, 'left'], [220, 'center'], [220, 'right']]) {
  add({ kind: 'wrap', x: 520, y, w, size: 12, lh: 16.8, text: para, align })
  y += 16.8 * 6 + 8
}
add({ kind: 'wrap', x: 520, y, w: 160, size: 12, lh: 16.8, text: 'path ~/Code/a-very-long-repository-name-without-breaks/src end' })
y += 16.8 * 3 + 8
add({ kind: 'wrap', x: 520, y, w: 160, size: 12, lh: 16.8, overflowWrap: 'anywhere', text: 'path ~/Code/a-very-long-repository-name-without-breaks/src end' })
y += 16.8 * 3 + 8
add({ kind: 'wrap', x: 520, y, w: 160, size: 12, lh: 16.8, wordBreak: 'break-all', text: 'path ~/Code/a-very-long-repository-name-without-breaks/src end' })
// 3. Line clamps with an ellipsis (folder names, 13px 500, centred, 2 lines; cards 4 lines).
y = 10
for (const [text, w] of [['quarterly-planning-and-roadmap-review-notes', 124], ['design system tokens and components library', 124], ['a b', 124], ['storefront checkout experiments archive 2026', 124]]) {
  add({ kind: 'clamp', x: 860, y, w, size: 13, weight: 500, lh: 16.9, align: 'center', clamp: 2, overflowWrap: 'anywhere', text })
  y += 16.9 * 2 + 10
}
add({ kind: 'clamp', x: 860, y, w: 200, size: 12, lh: 16.8, clamp: 4, overflowWrap: 'anywhere', text: para + ' ' + para })
y += 16.8 * 4 + 10
// 4. One line with text-overflow: ellipsis.
for (const w of [60, 75.5, 90, 120]) {
  add({ kind: 'ellipsis', x: 860, y, w, size: 13, lh: 18.2, ellipsis: true, text: 'storefront, docs-site and mobile-app' })
  y += 22
}
fs.writeFileSync(path.join(out, 'cases.json'), JSON.stringify(cases, null, 1))
const css = (c) => [
  `left:${c.x}px`, `top:${c.y}px`, c.w != null ? `width:${c.w}px` : '', `font-size:${c.size}px`, `font-weight:${c.weight}`,
  `line-height:${c.lh}px`, `text-align:${c.align}`,
  c.overflowWrap ? `overflow-wrap:${c.overflowWrap}` : '', c.wordBreak ? `word-break:${c.wordBreak}` : '',
  c.clamp ? `display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:${c.clamp};overflow:hidden` : '',
  c.ellipsis ? 'white-space:nowrap;overflow:hidden;text-overflow:ellipsis' : '',
  c.kind === 'stack' ? 'white-space:nowrap' : ''
].filter(Boolean).join(';')
const html = `<!doctype html><html><head><meta charset="utf-8"><style>
html, body { margin: 0; background: #fff; }
body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; -webkit-font-smoothing: antialiased; color: #000; }
.c { position: absolute; }
.bar { position: absolute; height: 2px; background: #000; }
</style></head><body>
${cases.map((c) => `<div class="c" style="${css(c)}">${c.text}</div>${c.bar ? `<div class="bar" style="left:${c.x + 30}px;top:${c.y + c.lh}px;width:20px"></div>` : ''}`).join('\n')}
<script>window.measure = () => [...document.querySelectorAll('.c')].map((e) => { const r = e.getBoundingClientRect(); return [r.top, r.height, r.width] })</script>
</body></html>`
fs.writeFileSync(path.join(out, 'page.html'), html)
console.log(cases.length, 'cases')
