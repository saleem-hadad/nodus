// usage: node seed.mjs <harness dir> <world dir> <theme> [mutation...]
// Writes the parity harness's synthetic world for one launch of the Rust app,
// with the state changes the shell's scenes make first.
import path from 'node:path'
const [harness, work, theme, ...mutations] = process.argv.slice(2)
const { seed, withExtra } = await import(path.join(path.resolve(harness), 'seed', 'index.mjs'))
const storefront = (s) => s.projects.find((p) => p.name === 'storefront').id
const MUTATIONS = {
  slowNaming: () => {},
  run: (s) => withExtra('run')(s),
  empty: (s) => {
    s.projects = []
    s.groups = []
    s.boards = {}
    s.captures = []
  },
  focusOn: (s) => {
    s.settings.focusFeature = true
    s.settings.focusMode = true
  },
  focusNone: (s) => s.projects.forEach((p) => delete p.focused),
  focusFeature: (s) => {
    s.settings.focusFeature = true
  },
  pinRail: (s) => {
    s.settings.pinProjectRail = true
  },
  capture: (s) => {
    s.settings.capture = true
  },
  zoomOut: (s) => {
    s.boards[storefront(s)].viewport = { x: 380, y: 230, zoom: 0.4 }
  },
  zoomIn: (s) => {
    s.boards[storefront(s)].viewport = { x: -1120, y: 40, zoom: 1.6 }
  }
}
seed(work, {
  theme,
  mutate: (state) => {
    for (const m of mutations) MUTATIONS[m](state)
  },
  // The stand-in claude is slow to name sections when a scene asks.
  responses: mutations.includes('slowNaming') ? { delayMs: 120_000 } : undefined
})
