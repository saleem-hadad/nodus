#!/usr/bin/env node
// Captures the golden screenshots of Nodus 0.9.4, which the Rust app matches:
// every scene in scenes/*.mjs, in light and dark, each from a fresh seeded
// world and a fresh launch of the Electron app.
//
//   node capture-electron.mjs [--scene id[,id…]] [--theme light|dark] [options]
//
// Options:
//   --scene a,b        only these scenes (a trailing * matches a prefix: board-*)
//   --theme t          light, dark or both (default both)
//   --out dir          where the PNGs go (default golden/ next to this file)
//   --app dir          the checkout of tag v0.9.4 to run (default: ../nodus-0.9.4 beside this repository;
//                      the Electron app left this repository at the cutover)
//   --build-dir dir    where it's built (default <app>/node_modules/.cache/parity)
//   --no-build         use the build that's there
//   --work dir         the seeded world (default: nodus-parity in macOS's Shared folder, DEFAULT_WORK)
//   --check            capture into a temporary folder and compare with --out instead of replacing it
//   --no-chromium-sandbox  for running inside another sandbox, where Chromium's can't start
//   --list             print the scenes
//   --write-md         regenerate scenes.md from the scene definitions
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { App, capture } from './lib/app.mjs'
import { comparePng } from './lib/pixels.mjs'
import { DEFAULT_WORK, seed } from './seed/index.mjs'
import { SCENES } from './scenes/index.mjs'
import { writeScenesMd } from './lib/scenes-md.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))

function parseArgs(argv) {
  const opts = {
    themes: ['light', 'dark'],
    out: path.join(here, 'golden'),
    app: path.resolve(here, '..', '..', '..', '..', '..', 'nodus-0.9.4'),
    build: true,
    work: DEFAULT_WORK,
    chromiumSandbox: true
  }
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    const next = () => argv[++i]
    if (a === '--scene') opts.scenes = next().split(',')
    else if (a === '--theme') opts.themes = next() === 'both' ? ['light', 'dark'] : [argv[i]]
    else if (a === '--out') opts.out = path.resolve(next())
    else if (a === '--app') opts.app = path.resolve(next())
    else if (a === '--build-dir') opts.buildDir = path.resolve(next())
    else if (a === '--no-build') opts.build = false
    else if (a === '--work') opts.work = path.resolve(next())
    else if (a === '--check') opts.check = true
    else if (a === '--no-chromium-sandbox') opts.chromiumSandbox = false
    else if (a === '--list') opts.list = true
    else if (a === '--write-md') opts.writeMd = true
    else throw new Error(`Unknown option ${a}`)
  }
  opts.buildDir ??= path.join(opts.app, 'node_modules', '.cache', 'parity')
  return opts
}

function selectScenes(patterns) {
  if (!patterns) return SCENES
  const picked = SCENES.filter((s) =>
    patterns.some((p) => (p.endsWith('*') ? s.id.startsWith(p.slice(0, -1)) : s.id === p))
  )
  if (!picked.length) throw new Error(`No scene matches ${patterns.join(', ')}`)
  return picked
}

function build(opts) {
  if (!fs.existsSync(path.join(opts.app, 'src', 'main', 'index.ts'))) {
    throw new Error(
      `${opts.app} isn't a checkout of the Electron app (no src/main/index.ts): pass --app <a checkout of tag v0.9.4>`
    )
  }
  console.log(`Building ${opts.app} into ${opts.buildDir}`)
  execFileSync(
    process.execPath,
    [path.join(opts.app, 'node_modules', 'electron-vite', 'bin', 'electron-vite.js'), 'build', '--outDir', opts.buildDir],
    { cwd: opts.app, stdio: ['ignore', 'ignore', 'inherit'] }
  )
}

/** One run at a time per world: two would share its folders and the session host. */
function lock(work) {
  fs.mkdirSync(work, { recursive: true })
  const file = path.join(work, '.lock')
  try {
    const pid = Number(fs.readFileSync(file, 'utf8'))
    process.kill(pid, 0)
    throw new Error(`Another capture (pid ${pid}) is using ${work}`)
  } catch (err) {
    if (err.code !== 'ENOENT' && err.code !== 'ESRCH') throw err
  }
  fs.writeFileSync(file, String(process.pid))
  return () => fs.rmSync(file, { force: true })
}

async function shoot(scene, theme, opts, dir) {
  const world = seed(opts.work, { theme, mutate: scene.seed, responses: scene.responses })
  const app = new App({ appDir: opts.app, buildDir: opts.buildDir, world, theme, chromiumSandbox: opts.chromiumSandbox })
  const file = path.join(dir, `${scene.id}.${theme}.png`)
  const started = Date.now()
  try {
    await app.launch()
    const { page = app.page, ...shot } = (await scene.run({ app, page: app.page, theme })) ?? {}
    const result = await capture(page, file, shot)
    return { scene: scene.id, theme, file, ms: Date.now() - started, ...result }
  } catch (err) {
    // What each window showed when the scene failed, to see where it got stuck.
    const dir = path.join(os.tmpdir(), 'nodus-parity-failures')
    fs.mkdirSync(dir, { recursive: true })
    for (const [i, w] of (app.electron?.windows() ?? []).entries()) {
      await w.screenshot({ path: path.join(dir, `${scene.id}.${theme}.${i}.png`) }).catch(() => {})
    }
    err.message = `${err.message.split('\n')[0]} (screens in ${dir})`
    throw err
  } finally {
    await app.close()
  }
}

async function main() {
  const opts = parseArgs(process.argv.slice(2))
  if (opts.list) {
    for (const s of SCENES) console.log(`${s.id.padEnd(34)} ${s.title}`)
    return
  }
  if (opts.writeMd) {
    writeScenesMd(SCENES, path.join(here, 'scenes.md'))
    console.log('Wrote scenes.md')
    return
  }
  const scenes = selectScenes(opts.scenes)
  if (opts.build) build(opts)
  const unlock = lock(opts.work)
  const dir = opts.check ? fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-parity-check-')) : opts.out
  fs.mkdirSync(dir, { recursive: true })
  const results = []
  try {
    for (const scene of scenes) {
      for (const theme of opts.themes) {
        let r
        // A launch now and then loses its window; a scene gets a second try before it counts as failed.
        for (let attempt = 1; attempt <= 2; attempt++) {
          try {
            r = await shoot(scene, theme, opts, dir)
          } catch (err) {
            r = { scene: scene.id, theme, error: err.message.split('\n')[0] }
          }
          if (!r.error && r.stable) break
        }
        results.push(r)
        const note = r.error ? `FAILED: ${r.error}` : r.stable ? '' : `unstable (still changing in ${JSON.stringify(r.changing)})`
        console.log(`${`${scene.id}.${theme}`.padEnd(42)} ${r.ms ? `${(r.ms / 1000).toFixed(1)}s` : ''} ${note}`)
      }
    }
  } finally {
    unlock()
  }

  if (opts.check) {
    console.log(`\nComparing ${dir} with ${opts.out}`)
    let differ = 0
    for (const r of results.filter((r) => r.file)) {
      const golden = path.join(opts.out, path.basename(r.file))
      if (!fs.existsSync(golden)) {
        console.log(`${path.basename(r.file).padEnd(46)} no golden`)
        continue
      }
      const c = comparePng(fs.readFileSync(golden), fs.readFileSync(r.file))
      if (c.diffPixels > 0) differ++
      const same = fs.readFileSync(golden).equals(fs.readFileSync(r.file))
      const verdict = same
        ? 'identical'
        : c.diffPixels === 0
          ? 'same to the eye (anti-aliasing noise only)'
          : `${c.diffPixels} px differ, within ${JSON.stringify(c.box)}`
      console.log(`${path.basename(r.file).padEnd(46)} ${verdict}`)
    }
    console.log(differ ? `\n${differ} shot(s) differ; this run's shots are in ${dir}.` : '\nEvery shot matches.')
    if (!differ && results.every((r) => r.file)) fs.rmSync(dir, { recursive: true, force: true })
  } else {
    // Shots taken before and not retaken keep their entries.
    const file = path.join(dir, 'report.json')
    let report = []
    try {
      report = JSON.parse(fs.readFileSync(file, 'utf8'))
    } catch {}
    const key = (r) => `${r.scene}.${r.theme}`
    const fresh = new Map(results.map(({ file, ...r }) => [key(r), r]))
    report = [...report.filter((r) => !fresh.has(key(r))), ...fresh.values()]
    const order = new Map(SCENES.map((s, i) => [s.id, i]))
    report.sort((a, b) => (order.get(a.scene) ?? 1e9) - (order.get(b.scene) ?? 1e9) || a.theme.localeCompare(b.theme))
    fs.writeFileSync(file, JSON.stringify(report, null, 2))
  }
  const failed = results.filter((r) => r.error || !r.stable)
  if (failed.length) {
    console.log(`\n${failed.length} shot(s) failed or never settled.`)
    process.exitCode = 1
  }
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
