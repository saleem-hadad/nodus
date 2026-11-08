#!/usr/bin/env node
// Compares screenshots with the goldens: two PNG files, or two folders of
// <scene>.<theme>.png (each candidate against the golden of the same name).
//
//   node migration/parity/diff.mjs <golden> <candidate> [--threshold 0.1] [--max-ratio 0] [--out <dir>]
//
// For each pair it prints how many pixels differ (pixelmatch: a perceptual
// YIQ colour distance above --threshold, anti-aliasing ignored), the share of
// the image, and the box around them. With --out, it writes <name>.diff.png
// (differences in red over a faded golden). It exits 1 when any pair differs
// in more than --max-ratio of its pixels, or has another size.
import fs from 'node:fs'
import path from 'node:path'
import { comparePng } from './lib/pixels.mjs'

function parse(argv) {
  const opts = { threshold: 0.1, maxRatio: 0, paths: [] }
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (a === '--threshold') opts.threshold = Number(argv[++i])
    else if (a === '--max-ratio') opts.maxRatio = Number(argv[++i])
    else if (a === '--out') opts.out = path.resolve(argv[++i])
    else opts.paths.push(path.resolve(a))
  }
  if (opts.paths.length !== 2) {
    console.error('Usage: node diff.mjs <golden> <candidate> [--threshold 0.1] [--max-ratio 0] [--out <dir>]')
    process.exit(2)
  }
  return opts
}

function pairs(golden, candidate) {
  if (!fs.statSync(golden).isDirectory()) return [[golden, candidate]]
  return fs
    .readdirSync(candidate)
    .filter((n) => n.endsWith('.png') && !n.endsWith('.diff.png'))
    .sort()
    .map((n) => [path.join(golden, n), path.join(candidate, n)])
}

const opts = parse(process.argv.slice(2))
if (opts.out) fs.mkdirSync(opts.out, { recursive: true })
let failed = 0
for (const [g, c] of pairs(...opts.paths)) {
  const name = path.basename(c)
  if (!fs.existsSync(g)) {
    console.log(`${name.padEnd(48)} no golden`)
    continue
  }
  const r = comparePng(fs.readFileSync(g), fs.readFileSync(c), { threshold: opts.threshold, diffImage: Boolean(opts.out) })
  if (!r.sameSize) {
    failed++
    console.log(`${name.padEnd(48)} size differs: golden ${r.size[0]}×${r.size[1]}, candidate ${r.size[2]}×${r.size[3]}`)
    continue
  }
  const bad = r.ratio > opts.maxRatio
  if (bad) failed++
  const box = r.box ? ` in ${r.box.w}×${r.box.h} at ${r.box.x},${r.box.y}` : ''
  console.log(`${name.padEnd(48)} ${r.diffPixels ? `${r.diffPixels} px (${(r.ratio * 100).toFixed(3)}%)${box}` : 'same'}${bad ? '  ✗' : ''}`)
  if (opts.out && r.diffPixels) fs.writeFileSync(path.join(opts.out, name.replace(/\.png$/, '.diff.png')), r.diff)
}
if (failed) {
  console.log(`\n${failed} differ beyond --max-ratio ${opts.maxRatio}.`)
  process.exit(1)
}
