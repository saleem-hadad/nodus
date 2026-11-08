// Writes scenes.md from the scene definitions, so the list and the code that
// draws each scene never disagree.
import fs from 'node:fs'

export function writeScenesMd(scenes, file) {
  const areas = []
  for (const s of scenes) {
    let area = areas.find((a) => a.name === s.area)
    if (!area) areas.push((area = { name: s.area, scenes: [] }))
    area.scenes.push(s)
  }
  const out = []
  out.push('# Parity scenes')
  out.push('')
  out.push(
    'Every screen and state of Nodus 0.9.4 that has a golden screenshot, in light and dark. Generated from `scenes/*.mjs` by `node capture-electron.mjs --write-md`; edit the scene definitions, not this file.'
  )
  out.push('')
  out.push(
    'Each scene starts from the same seeded world (`seed/`, `README.md` § The world) and a fresh launch of the app, 1440×900 CSS px at 2x, clock frozen at 2026-10-20 15:00 UTC. Its shot is `golden/<id>.<light|dark>.png`. "Live" means the session\'s stand-in process is running; statuses come from hooks posted to the session host, as Claude Code\'s would be.'
  )
  out.push('')
  out.push(`${scenes.length} scenes × 2 themes = ${scenes.length * 2} shots.`)
  out.push('')
  for (const area of areas) {
    out.push(`## ${area.name}`)
    out.push('')
    out.push('| Scene | Shows | Specs | Steps |')
    out.push('| --- | --- | --- | --- |')
    for (const s of area.scenes) {
      const steps = s.steps.map((step, i) => `${i + 1}. ${step}`).join('<br>')
      out.push(`| \`${s.id}\` | ${s.title} | ${s.specs.join(', ')} | ${steps} |`)
    }
    out.push('')
  }
  fs.writeFileSync(file, out.join('\n'))
}
