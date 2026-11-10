// Comparing screenshots: a perceptual pixel diff (pixelmatch's YIQ colour
// distance with anti-aliasing detection) and the box around what differs.
import { PNG } from 'pngjs'
import pixelmatch from 'pixelmatch'

export const readPng = (buf) => PNG.sync.read(buf)

/**
 * Compares two PNG buffers. `threshold` is pixelmatch's per-pixel colour
 * tolerance (0–1); anti-aliased pixels don't count as different.
 */
export function comparePng(a, b, { threshold = 0.1, diffImage = false } = {}) {
  const A = readPng(a)
  const B = readPng(b)
  if (A.width !== B.width || A.height !== B.height) {
    return { sameSize: false, size: [A.width, A.height, B.width, B.height], diffPixels: Infinity, ratio: 1 }
  }
  const { width, height } = A
  const out = diffImage ? new PNG({ width, height }) : null
  const diffPixels = pixelmatch(A.data, B.data, out?.data ?? null, width, height, { threshold, includeAA: false })
  return {
    sameSize: true,
    width,
    height,
    diffPixels,
    ratio: diffPixels / (width * height),
    box: diffPixels ? changedBox(A, B) : null,
    diff: out ? PNG.sync.write(out) : null
  }
}

/** How many pixels aren't byte-identical. */
export function countChanged(A, B) {
  let n = 0
  for (let i = 0; i < A.data.length; i += 4) {
    if (A.data[i] !== B.data[i] || A.data[i + 1] !== B.data[i + 1] || A.data[i + 2] !== B.data[i + 2] || A.data[i + 3] !== B.data[i + 3]) n++
  }
  return n
}

/** The smallest box holding every pixel that isn't byte-identical, in device pixels. */
export function changedBox(A, B) {
  let x0 = Infinity
  let y0 = Infinity
  let x1 = -1
  let y1 = -1
  for (let y = 0; y < A.height; y++) {
    for (let x = 0; x < A.width; x++) {
      const i = (y * A.width + x) * 4
      if (A.data[i] !== B.data[i] || A.data[i + 1] !== B.data[i + 1] || A.data[i + 2] !== B.data[i + 2] || A.data[i + 3] !== B.data[i + 3]) {
        if (x < x0) x0 = x
        if (x > x1) x1 = x
        if (y < y0) y0 = y
        if (y > y1) y1 = y
      }
    }
  }
  return x1 < 0 ? null : { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 }
}
