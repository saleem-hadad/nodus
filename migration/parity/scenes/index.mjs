// Every scene, in the order they're captured and listed in scenes.md.
import projects from './projects.mjs'
import board from './board.mjs'
import overlays from './overlays.mjs'
import panels from './panels.mjs'
import capture from './capture.mjs'

export const SCENES = [...projects, ...board, ...overlays, ...panels, ...capture]

const seen = new Set()
for (const s of SCENES) {
  if (seen.has(s.id)) throw new Error(`Two scenes are called ${s.id}`)
  seen.add(s.id)
  for (const key of ['area', 'title', 'specs', 'steps', 'run']) {
    if (!s[key]) throw new Error(`Scene ${s.id} has no ${key}`)
  }
}
