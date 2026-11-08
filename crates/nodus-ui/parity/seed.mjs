// usage: node seed.mjs <harness dir> <world dir> <theme> [mutation...]
// Writes the parity harness's synthetic world for one launch of the Rust app,
// with the state changes the shell's scenes make first.
import path from 'node:path'
const [harness, work, theme, ...mutations] = process.argv.slice(2)
const { seed } = await import(path.join(path.resolve(harness), 'seed', 'index.mjs'))
const MUTATIONS = {
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
  }
}
seed(work, {
  theme,
  mutate: (state) => {
    for (const m of mutations) MUTATIONS[m](state)
  }
})
