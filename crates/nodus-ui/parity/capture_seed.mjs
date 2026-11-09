// usage: node capture_seed.mjs <harness dir> <world dir> <theme> '<spec json>'
// Writes the parity harness's synthetic world for one capture scene, as
// migration/parity/scenes/capture.mjs seeds it: `{"noCaptures": true}` empties
// the recent captures, `{"responses": {…}}` changes what the claude stand-in
// answers. Prints the ids and the synthetic screenshot thumbnail the scenes use.
import path from 'node:path'
import { createRequire } from 'node:module'
const [harness, work, theme, raw] = process.argv.slice(2)
const spec = JSON.parse(raw || '{}')
const root = path.resolve(harness)
const { seed } = await import(path.join(root, 'seed', 'index.mjs'))
const { projectId, sessionId } = await import(path.join(root, 'seed', 'world.mjs'))
// capture-receipt-note: the stand-in guesses a note in storefront.
const responses = spec.guessNote
  ? {
      ...spec.responses,
      guess: { kind: 'note', project: projectId('storefront'), session: '', reason: 'It’s a reminder, not work to start now.' }
    }
  : spec.responses
seed(work, {
  theme,
  responses,
  mutate: (state) => {
    if (spec.noCaptures) state.captures = []
  }
})

/** The harness's made-up screenshot: a browser-ish page of grey blocks. */
function thumb() {
  const require = createRequire(path.join(process.env.NODE_PATH ?? process.cwd(), 'x.js'))
  const { PNG } = require('pngjs')
  const w = 360
  const h = 225
  const png = new PNG({ width: w, height: h })
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4
      let c = [246, 246, 244]
      if (y < 22) c = [228, 228, 226]
      else if (y > 40 && y < 70 && x > 20 && x < 220) c = [60, 60, 64]
      else if (y > 90 && y < 190 && x > 20 && x < 160) c = [120, 170, 240]
      else if (y > 90 && y < 190 && x > 180 && x < 340) c = [250, 200, 120]
      png.data.set([...c, 255], i)
    }
  }
  return `data:image/png;base64,${PNG.sync.write(png).toString('base64')}`
}

console.log(
  JSON.stringify({
    storefront: projectId('storefront'),
    sfTax: sessionId('sf-tax'),
    capNew: sessionId('cap-new-session'),
    thumb: spec.thumb ? thumb() : null
  })
)
