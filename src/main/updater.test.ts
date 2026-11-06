import crypto from 'crypto'
import { EventEmitter } from 'events'
import fs from 'fs'
import os from 'os'
import path from 'path'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'

// The Windows updater: an installer is downloaded, kept until the user restarts into it, and run.
const mocks = vi.hoisted(() => ({
  userData: '',
  quit: vi.fn(),
  showMessageBox: vi.fn(),
  liveSessionIds: vi.fn((): string[] => []),
  stopAll: vi.fn(async () => {}),
  spawn: vi.fn()
}))

vi.mock('electron', () => ({
  app: {
    getVersion: () => '0.8.9',
    getPath: () => mocks.userData,
    isPackaged: true,
    runningUnderARM64Translation: false,
    quit: mocks.quit,
    on: vi.fn()
  },
  dialog: { showMessageBox: mocks.showMessageBox }
}))
vi.mock('./pty', () => ({ liveSessionIds: mocks.liveSessionIds, stopAll: mocks.stopAll }))
vi.mock('./store', () => ({ getState: () => ({ settings: { autoUpdate: false } }) }))
vi.mock('child_process', async (importOriginal) => ({
  ...(await importOriginal<typeof import('child_process')>()),
  spawn: mocks.spawn
}))

const platform = process.platform
const ASSET = `Nodus-Setup-${process.arch === 'arm64' ? 'arm64' : 'x64'}.exe`
const DOWNLOAD_URL = `https://github.com/saleem-hadad/nodus/releases/download/v0.9.0/${ASSET}`
const installer = Buffer.from('the 0.9.0 installer')
const sha256 = (data: Buffer) => crypto.createHash('sha256').update(data).digest('hex')

let downloads = 0
/** GitHub: the latest release is 0.9.0, with an installer for this machine unless `assets` is false. */
function serveRelease({ assets = true } = {}) {
  downloads = 0
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string) => {
      if (url === DOWNLOAD_URL) {
        downloads++
        return new Response(installer)
      }
      const asset = {
        name: ASSET,
        state: 'uploaded',
        size: installer.length,
        digest: `sha256:${sha256(installer)}`,
        browser_download_url: DOWNLOAD_URL
      }
      return new Response(JSON.stringify({ tag_name: 'v0.9.0', assets: assets ? [asset] : [] }))
    })
  )
}

/** The updater as a fresh launch starts it, on Windows. */
async function launch() {
  vi.resetModules()
  const updater = await import('./updater')
  updater.startUpdater(() => {})
  return updater
}

async function staged() {
  const updater = await launch()
  await updater.check(true)
  await vi.waitFor(() => expect(updater.getStatus().state).toBe('ready'))
  return updater
}

/** A started installer: Windows starts it, or it fails to start. */
function child(outcome: 'spawn' | 'error') {
  const proc = Object.assign(new EventEmitter(), { unref: vi.fn() })
  setImmediate(() => (outcome === 'spawn' ? proc.emit('spawn') : proc.emit('error', new Error('blocked'))))
  return proc
}

const updates = () => path.join(mocks.userData, 'updates')
const installerPath = () => path.join(updates(), 'Nodus-Setup-0.9.0.exe')

beforeAll(() => {
  Object.defineProperty(process, 'platform', { value: 'win32' })
})

afterAll(() => {
  Object.defineProperty(process, 'platform', { value: platform })
})

beforeEach(() => {
  mocks.userData = fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-updater-'))
  vi.spyOn(console, 'error').mockImplementation(() => {})
  serveRelease()
})

afterEach(() => {
  vi.clearAllMocks()
  vi.unstubAllGlobals()
  fs.rmSync(mocks.userData, { recursive: true, force: true })
})

describe('the installer between launches', () => {
  it('is downloaded once, and kept for the next launch', async () => {
    await staged()
    expect(downloads).toBe(1)
    await staged()
    expect(downloads).toBe(1)
    expect(fs.readFileSync(installerPath())).toEqual(installer)
  })

  it('is downloaded again when the kept one isn’t the release’s', async () => {
    fs.mkdirSync(updates(), { recursive: true })
    fs.writeFileSync(installerPath(), Buffer.from('half a download'))
    await staged()
    expect(downloads).toBe(1)
    expect(fs.readFileSync(installerPath())).toEqual(installer)
  })

  it('only the newest newer one is kept', async () => {
    serveRelease({ assets: false })
    fs.mkdirSync(updates(), { recursive: true })
    for (const name of ['Nodus-Setup-0.8.0.exe', 'Nodus-Setup-0.8.9.exe', 'Nodus-Setup-0.9.0.exe', 'Nodus-Setup-0.9.1.exe', 'stray.tmp']) {
      fs.writeFileSync(path.join(updates(), name), '')
    }
    const updater = await launch()
    await updater.check(true)
    expect(fs.readdirSync(updates())).toEqual(['Nodus-Setup-0.9.1.exe'])
  })
})

describe('Restart to Update on Windows', () => {
  beforeEach(() => {
    mocks.liveSessionIds.mockReturnValue(['s1'])
    mocks.showMessageBox.mockResolvedValue({ response: 0 })
  })

  it('stops the sessions and quits once Windows has started the installer', async () => {
    const updater = await staged()
    mocks.spawn.mockImplementation(() => child('spawn'))
    await updater.restartToUpdate()
    expect(mocks.stopAll).toHaveBeenCalledOnce()
    expect(mocks.spawn).toHaveBeenCalledWith(installerPath(), ['--updated', '/S', '--force-run'], {
      detached: true,
      stdio: 'ignore'
    })
    expect(mocks.quit).toHaveBeenCalledOnce()
    expect(mocks.stopAll.mock.invocationCallOrder[0]).toBeLessThan(mocks.spawn.mock.invocationCallOrder[0])
  })

  it('stays open and says so when the installer doesn’t start', async () => {
    const updater = await staged()
    mocks.spawn.mockImplementation(() => child('error'))
    await updater.restartToUpdate()
    expect(mocks.quit).not.toHaveBeenCalled()
    expect(updater.getStatus()).toMatchObject({ state: 'error' })
    expect(fs.existsSync(installerPath())).toBe(false)
    // Check for Updates gets it again.
    await updater.check(true)
    await vi.waitFor(() => expect(updater.getStatus().state).toBe('ready'))
    expect(downloads).toBe(2)
  })

  it('stops nothing when the installer has gone', async () => {
    const updater = await staged()
    fs.rmSync(installerPath())
    await updater.restartToUpdate()
    expect(mocks.showMessageBox).not.toHaveBeenCalled()
    expect(mocks.stopAll).not.toHaveBeenCalled()
    expect(mocks.quit).not.toHaveBeenCalled()
    expect(updater.getStatus()).toMatchObject({ state: 'error' })
  })

  it('leaves the sessions running when the user says not now', async () => {
    const updater = await staged()
    mocks.showMessageBox.mockResolvedValue({ response: 1 })
    await updater.restartToUpdate()
    expect(mocks.stopAll).not.toHaveBeenCalled()
    expect(mocks.spawn).not.toHaveBeenCalled()
    expect(updater.getStatus()).toMatchObject({ state: 'ready' })
  })

  it('asks once when the button is clicked twice', async () => {
    const updater = await staged()
    mocks.spawn.mockImplementation(() => child('spawn'))
    await Promise.all([updater.restartToUpdate(), updater.restartToUpdate()])
    expect(mocks.showMessageBox).toHaveBeenCalledOnce()
    expect(mocks.spawn).toHaveBeenCalledOnce()
  })
})
