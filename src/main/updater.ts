// Keeps the app up to date from GitHub releases.
//
// Early releases were ad-hoc signed, and Squirrel.Mac (what electron-updater uses)
// only installs updates signed with a Developer ID. So this does the same job
// by hand: it finds the latest release, downloads the DMG for this Mac, checks
// it against the digest GitHub keeps for the file, copies the app out of it
// and, once the app quits, swaps it in for the running one.
import { app } from 'electron'
import { execFile, spawn } from 'child_process'
import crypto from 'crypto'
import fs from 'fs'
import path from 'path'
import { Readable, Transform } from 'stream'
import { pipeline } from 'stream/promises'
import type { ReadableStream as WebReadableStream } from 'stream/web'
import type { UpdateState, UpdateStatus } from '@shared/types'
import { getState } from './store'

/** The newest release that isn't a draft or a prerelease. */
const LATEST_URL = 'https://api.github.com/repos/saleem-hadad/nodus/releases/latest'
/** A little after launch, so the first check never competes with opening the board. */
const FIRST_CHECK_MS = 30_000
const CHECK_EVERY_MS = 4 * 60 * 60 * 1000
const LOOKUP_TIMEOUT_MS = 20_000
/** A download that sends nothing for this long is given up. */
const STALL_MS = 60_000

/** An error whose message is fit to show in Settings. */
class UpdateError extends Error {}

let status: UpdateStatus = { state: 'idle', current: app.getVersion() }
let onChange: (status: UpdateStatus) => void = () => {}
/** Leftovers from earlier runs are cleared before the first check. */
let cleaned: Promise<void> = Promise.resolve()
/** A lookup or download is under way. */
let busy = false
/** The new version's app, copied out of its DMG and waiting to be swapped in. */
let staged: { version: string; app: string } | null = null
/** A version that failed to install on an earlier quit. It isn't downloaded again unless the user asks. */
let failedVersion: string | null = null
/** Open the new version once it's swapped in (Restart to Update), rather than only installing on quit. */
let relaunch = false
let swapping = false

function setState(next: UpdateState) {
  status = { ...next, current: app.getVersion() }
  onChange(status)
}

export const getStatus = (): UpdateStatus => status

const updatesDir = () => path.join(app.getPath('userData'), 'updates')
const failedFile = () => path.join(updatesDir(), 'failed')
/** …/Nodus.app/Contents/MacOS/Nodus → …/Nodus.app */
const bundlePath = () => path.resolve(process.execPath, '../../..')
/** Apple silicon gets the arm64 DMG, even when this copy is the Intel one. Names match electron-builder.yml. */
const dmgName = () => `Nodus-${process.arch === 'arm64' || app.runningUnderARM64Translation ? 'arm64' : 'x64'}.dmg`

/** Why this copy of the app can't replace itself, or null when it can. */
function whyOff(): string | null {
  if (process.platform !== 'darwin') return 'Updates are only built for macOS.'
  if (!app.isPackaged) return 'Development builds don’t update themselves.'
  const bundle = bundlePath()
  try {
    // The swap renames the app, so it needs write access to the app and to its folder.
    // A copy running from the DMG, or one macOS translocated, has neither.
    fs.accessSync(bundle, fs.constants.W_OK)
    fs.accessSync(path.dirname(bundle), fs.constants.W_OK)
    return null
  } catch {
    return app.isInApplicationsFolder()
      ? 'Your account can’t change apps in Applications, so Nodus can’t update itself.'
      : 'Move Nodus to your Applications folder so it can update itself.'
  }
}

/** Whether version a comes after b, by major.minor.patch. */
function isNewer(a: string, b: string): boolean {
  const parts = (v: string) => v.split('.').map((n) => parseInt(n, 10) || 0)
  const [x, y] = [parts(a), parts(b)]
  for (let i = 0; i < 3; i++) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0)
  }
  return false
}

function run(command: string, args: string[]): Promise<{ stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    execFile(command, args, { encoding: 'utf8' }, (err, stdout, stderr) =>
      err ? reject(Object.assign(err, { stderr })) : resolve({ stdout, stderr })
    )
  })
}

/** Electron's fs reads .asar files as folders, so it can't delete an app's app.asar; rm can. */
const remove = (target: string) => run('/bin/rm', ['-rf', target])

interface Release {
  version: string
  url: string
  size: number
  /** Hex SHA-256 that GitHub computed for the DMG. */
  sha256: string | null
}

interface GitHubAsset {
  name: string
  state: string
  size: number
  digest?: string | null
  browser_download_url: string
}

/** The latest release, or null while it has no finished DMG for this Mac (it may still be uploading). */
async function latestRelease(): Promise<Release | null> {
  let res: Response
  try {
    res = await fetch(LATEST_URL, {
      headers: { Accept: 'application/vnd.github+json', 'User-Agent': `Nodus/${app.getVersion()}` },
      signal: AbortSignal.timeout(LOOKUP_TIMEOUT_MS)
    })
  } catch {
    throw new UpdateError('Couldn’t reach GitHub to check for updates.')
  }
  // Calls without a token get 60 an hour per address, which a shared network can use up.
  if (res.status === 403 || res.status === 429) {
    throw new UpdateError('GitHub is limiting update checks right now. Try again later.')
  }
  if (!res.ok) throw new UpdateError(`GitHub answered the update check with an error (${res.status}).`)
  const body = (await res.json()) as { tag_name?: string; assets?: GitHubAsset[] }
  const asset = body.assets?.find((a) => a.name === dmgName() && a.state === 'uploaded')
  // The version names a folder that gets deleted, so it has to be a plain one.
  const version = /^v?(\d+\.\d+\.\d+)$/.exec(body.tag_name ?? '')?.[1]
  if (!version || !asset) return null
  return {
    version,
    url: asset.browser_download_url,
    size: asset.size,
    sha256: /^sha256:([0-9a-f]{64})$/.exec(asset.digest ?? '')?.[1] ?? null
  }
}

async function download(release: Release, file: string, progress: (percent: number) => void) {
  const controller = new AbortController()
  const stall = setTimeout(() => controller.abort(), STALL_MS)
  try {
    const res = await fetch(release.url, {
      headers: { 'User-Agent': `Nodus/${app.getVersion()}` },
      signal: controller.signal
    })
    if (!res.ok || !res.body) throw new UpdateError(`GitHub answered the download with an error (${res.status}).`)
    const hash = crypto.createHash('sha256')
    let received = 0
    let shown = 0
    const meter = new Transform({
      transform(chunk: Buffer, _encoding, done) {
        stall.refresh()
        received += chunk.length
        hash.update(chunk)
        const percent = Math.min(100, Math.floor((received / release.size) * 100))
        if (percent !== shown) progress((shown = percent))
        done(null, chunk)
      }
    })
    await pipeline(Readable.fromWeb(res.body as WebReadableStream), meter, fs.createWriteStream(file))
    if (received !== release.size || (release.sha256 && hash.digest('hex') !== release.sha256)) {
      throw new UpdateError('The update came through damaged. Try again later.')
    }
  } catch (err) {
    if (err instanceof UpdateError) throw err
    throw new UpdateError('The update didn’t finish downloading. Check your connection.')
  } finally {
    clearTimeout(stall)
  }
}

/** Copies the app out of the DMG, mounted where Finder doesn't show it. */
async function copyApp(dmg: string, target: string) {
  const mount = await fs.promises.mkdtemp(path.join(updatesDir(), 'mount-'))
  try {
    await run('/usr/bin/hdiutil', ['attach', dmg, '-nobrowse', '-noautoopen', '-readonly', '-mountpoint', mount])
    try {
      const name = (await fs.promises.readdir(mount)).find((n) => n.endsWith('.app'))
      if (!name) throw new UpdateError('The update has no app in it.')
      // --noqtn: a quarantine flag would have Gatekeeper stop the new version on its first launch.
      await run('/usr/bin/ditto', ['--noqtn', path.join(mount, name), target])
    } finally {
      await run('/usr/bin/hdiutil', ['detach', mount, '-force']).catch((err) =>
        console.error('[updater] detach failed', err)
      )
    }
  } finally {
    await fs.promises.rmdir(mount).catch(() => {})
  }
}

const plistValue = async (appPath: string, key: string) =>
  (await run('/usr/bin/plutil', ['-extract', key, 'raw', '-o', '-', path.join(appPath, 'Contents', 'Info.plist')]))
    .stdout.trim()

/** The team that signed an app, or null for an ad-hoc signature. */
async function teamId(appPath: string): Promise<string | null> {
  const { stderr } = await run('/usr/bin/codesign', ['-dv', appPath])
  const team = /^TeamIdentifier=(.+)$/m.exec(stderr)?.[1]
  return team && team !== 'not set' ? team : null
}

/** Makes sure the new app is intact, is this app, and is the version GitHub named. */
async function verify(appPath: string, version: string) {
  try {
    await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', appPath])
  } catch {
    throw new UpdateError('The new version’s signature didn’t check out.')
  }
  const [id, ownId, newVersion] = await Promise.all([
    plistValue(appPath, 'CFBundleIdentifier'),
    plistValue(bundlePath(), 'CFBundleIdentifier'),
    plistValue(appPath, 'CFBundleShortVersionString')
  ])
  if (id !== ownId || newVersion !== version) throw new UpdateError('The update isn’t the expected version of Nodus.')
  // Once releases are signed with a Developer ID, only one from the same team may replace this copy.
  const team = await teamId(bundlePath())
  if (team && (await teamId(appPath)) !== team) throw new UpdateError('The update isn’t signed by the same developer.')
}

/** Downloads a release and stages its app for the swap. */
async function prepare(release: Release) {
  const dir = path.join(updatesDir(), release.version)
  const dmg = path.join(updatesDir(), `Nodus-${release.version}.dmg`)
  const target = path.join(dir, 'Nodus.app')
  setState({ state: 'downloading', version: release.version, percent: 0 })
  try {
    await remove(dir)
    await fs.promises.mkdir(dir, { recursive: true })
    await download(release, dmg, (percent) => setState({ state: 'downloading', version: release.version, percent }))
    await copyApp(dmg, target)
    await verify(target, release.version)
    staged = { version: release.version, app: target }
    setState({ state: 'ready', version: release.version })
  } catch (err) {
    console.error('[updater] update failed', err)
    await remove(dir).catch(() => {})
    const message = err instanceof UpdateError ? err.message : 'The update couldn’t be prepared. Try again later.'
    setState({ state: 'error', message })
  } finally {
    await fs.promises.rm(dmg, { force: true }).catch(() => {})
  }
}

/**
 * Looks for a newer release and starts downloading it. Resolves once the
 * lookup is done; the download carries on. `asked` is a check the user
 * started, which also retries a version that failed to install before.
 */
export async function check(asked = false): Promise<void> {
  await cleaned
  if (busy || staged || status.state === 'off') return
  busy = true
  setState({ state: 'checking' })
  let release: Release | null
  try {
    release = await latestRelease()
  } catch (err) {
    busy = false
    return setState({ state: 'error', message: (err as Error).message })
  }
  if (!release || !isNewer(release.version, app.getVersion())) {
    busy = false
    return setState({ state: 'latest' })
  }
  if (release.version === failedVersion && !asked) {
    busy = false
    return setState({ state: 'error', message: `Nodus ${release.version} downloaded but couldn’t install itself.` })
  }
  void prepare(release).finally(() => {
    busy = false
  })
}

/** Quits, swaps the new version in and opens it. Sessions keep running in the session host. */
export function restartToUpdate() {
  if (!staged) return
  relaunch = true
  app.quit()
}

/**
 * Runs once the app has exited, since a running app can't replace itself:
 * moves the running copy aside, moves the new one into its place and deletes
 * the old. If the new one can't go in, the old one goes back, and the version
 * is written down so it isn't downloaded again on every launch. The app opens
 * again either way when the user chose Restart to Update.
 */
const SWAP_SCRIPT = `
trap '' HUP INT TERM
while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
app=$2 new=$3 old=$4
rm -rf "$old"
if mv "$app" "$old"; then
  if mv "$new" "$app"; then
    rm -rf "$old"
  else
    rm -rf "$app"
    mv "$old" "$app"
    echo "$6" > "$5"
  fi
else
  echo "$6" > "$5"
fi
[ -n "$7" ] || exit 0
if [ -n "$8" ]; then open -n --env "NODUS_USER_DATA=$8" "$app"; else open -n "$app"; fi
`

function swapOnExit() {
  if (!staged || swapping) return
  swapping = true
  const args = [
    String(process.pid),
    bundlePath(),
    staged.app,
    path.join(updatesDir(), 'previous.app'),
    failedFile(),
    staged.version,
    relaunch ? '1' : '',
    // A test copy keeps its own data once it opens again.
    relaunch ? (process.env.NODUS_USER_DATA ?? '') : ''
  ]
  spawn('/bin/sh', ['-c', SWAP_SCRIPT, 'nodus-update', ...args], { detached: true, stdio: 'ignore' }).unref()
}

/** Clears what earlier runs left behind: downloads, a staged app that never went in, the previous version. */
async function cleanUp() {
  const dir = updatesDir()
  try {
    const failed = (await fs.promises.readFile(failedFile(), 'utf8')).trim()
    // Once this copy has caught up with that version, the failure no longer matters.
    if (isNewer(failed, app.getVersion())) failedVersion = failed
  } catch {
    // Nothing failed.
  }
  for (const name of await fs.promises.readdir(dir).catch(() => [])) {
    if (name === 'failed' && failedVersion) continue
    // A DMG still mounted from a quit mid-copy.
    if (name.startsWith('mount-')) await run('/usr/bin/hdiutil', ['detach', path.join(dir, name), '-force']).catch(() => {})
    await remove(path.join(dir, name)).catch((err) => console.error('[updater] cleanup failed', err))
  }
}

/** Checks a little after launch and every few hours after, while Settings allows it. */
export function startUpdater(notify: (status: UpdateStatus) => void) {
  onChange = notify
  const reason = whyOff()
  if (reason) return setState({ state: 'off', reason })
  cleaned = cleanUp()
  app.on('will-quit', swapOnExit)
  const autoCheck = () => {
    if (getState().settings.autoUpdate) void check()
  }
  setTimeout(autoCheck, FIRST_CHECK_MS)
  setInterval(autoCheck, CHECK_EVERY_MS)
}
