// What floating capture brings from the user's screen: the app in front and
// its browser tab, a screenshot, and what's on the clipboard.
import { execFile } from 'child_process'
import crypto from 'crypto'
import { app, clipboard, desktopCapturer, nativeImage, screen } from 'electron'
import fs from 'fs'
import path from 'path'
import { MAX_COPIED_CHARS } from '@shared/capture'
import type { CaptureAttachment } from '@shared/types'

const isMac = process.platform === 'darwin'

/** Screenshots and copied images older than this are cleared away at launch. */
const KEEP_MS = 30 * 24 * 60 * 60 * 1000

export interface FrontApp {
  name: string
  bundleId: string
}

function run(file: string, args: string[], timeout: number): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(file, args, { timeout, windowsHide: true }, (err, stdout) => (err ? reject(err) : resolve(stdout)))
  })
}

/** Reads `lsappinfo info` output: the app's name on the first line, then bundleID="…". */
export function parseAppInfo(out: string): FrontApp | null {
  const bundleId = /bundleID="([^"]+)"/.exec(out)?.[1]
  if (!bundleId) return null
  const name = /^\s*"([^"]+)"\s+ASN:/m.exec(out)?.[1] ?? bundleId.split('.').pop()!
  return { name, bundleId }
}

/** The app in front, on a Mac. lsappinfo needs no permission and answers in a few milliseconds. */
export async function frontApp(): Promise<FrontApp | null> {
  if (!isMac) return null
  try {
    const asn = (await run('lsappinfo', ['front'], 1000)).trim()
    if (!asn) return null
    return parseAppInfo(await run('lsappinfo', ['info', '-only', 'bundleid', '-only', 'name', asn], 1000))
  } catch {
    return null
  }
}

/** Browsers whose front tab AppleScript can read. Firefox has no way to. */
const SAFARI = new Set(['com.apple.Safari', 'com.apple.SafariTechnologyPreview'])
const CHROMIUM = new Set([
  'com.google.Chrome',
  'com.google.Chrome.beta',
  'com.google.Chrome.canary',
  'org.chromium.Chromium',
  'company.thebrowser.Browser',
  'company.thebrowser.dia',
  'com.brave.Browser',
  'com.microsoft.edgemac',
  'com.vivaldi.Vivaldi',
  'com.operasoftware.Opera'
])

export const isBrowser = (bundleId: string) => SAFARI.has(bundleId) || CHROMIUM.has(bundleId)

/** The script that reads a browser's front tab: its URL, then its title, a line each. */
export function tabScript(bundleId: string): string | null {
  // Bundle ids are dotted names; anything else never reaches a script.
  if (!/^[\w.-]+$/.test(bundleId)) return null
  if (SAFARI.has(bundleId))
    return `tell application id "${bundleId}" to tell front document to return (URL as text) & linefeed & (name as text)`
  if (CHROMIUM.has(bundleId))
    return `tell application id "${bundleId}" to tell active tab of front window to return (URL as text) & linefeed & (title as text)`
  return null
}

/**
 * The front tab of a browser in front. macOS asks once, for each browser,
 * whether Nodus may read it; until it may, there's no tab.
 */
export async function browserTab(front: FrontApp): Promise<Extract<CaptureAttachment, { kind: 'tab' }> | null> {
  const script = tabScript(front.bundleId)
  if (!script) return null
  try {
    // Long enough for the user to answer macOS asking the first time.
    const [url = '', ...title] = (await run('osascript', ['-e', script], 60_000)).trim().split('\n')
    if (!/^https?:\/\//i.test(url)) return null
    return { kind: 'tab', url, title: title.join(' ').trim(), app: front.name }
  } catch {
    return null
  }
}

/** Where screenshots and copied images are kept. */
function shotsDir() {
  const dir = path.join(app.getPath('userData'), 'captures')
  fs.mkdirSync(dir, { recursive: true })
  return dir
}

const newShotPath = () => path.join(shotsDir(), `${crypto.randomUUID()}.png`)

const thumbOf = (image: Electron.NativeImage) => {
  const { width } = image.getSize()
  return (width > 360 ? image.resize({ width: 360, quality: 'good' }) : image).toDataURL()
}

/**
 * A screenshot the user takes: on a Mac, macOS's own crosshair (Space for a
 * window), and off a Mac the whole screen the pointer is on. Null if they
 * cancel. The panel is hidden first, so it isn't in the shot.
 */
export async function takeScreenshot(): Promise<Extract<CaptureAttachment, { kind: 'image' }> | null> {
  const file = newShotPath()
  if (isMac) {
    // -i lets the user choose, -x keeps it quiet, -o leaves out a window's shadow.
    await run('screencapture', ['-i', '-x', '-o', file], 5 * 60_000).catch(() => '')
  } else {
    const display = screen.getDisplayNearestPoint(screen.getCursorScreenPoint())
    const size = {
      width: Math.round(display.size.width * display.scaleFactor),
      height: Math.round(display.size.height * display.scaleFactor)
    }
    const sources = await desktopCapturer.getSources({ types: ['screen'], thumbnailSize: size })
    const source = sources.find((s) => s.display_id === String(display.id)) ?? sources[0]
    if (source && !source.thumbnail.isEmpty()) fs.writeFileSync(file, source.thumbnail.toPNG())
  }
  if (!fs.existsSync(file)) return null
  const image = nativeImage.createFromPath(file)
  if (image.isEmpty()) return null
  return { kind: 'image', path: file, thumb: thumbOf(image) }
}

/** A copied image, such as a screenshot taken to the clipboard. */
async function clipboardImage(): Promise<Electron.NativeImage | null> {
  try {
    for (const item of await clipboard.read()) {
      const type = item.types.find((t) => t.startsWith('image/'))
      if (!type) continue
      const blob = (await item.getType(type)) as Blob
      const image = nativeImage.createFromBuffer(Buffer.from(await blob.arrayBuffer()))
      if (!image.isEmpty()) return image
    }
  } catch {
    // Nothing readable.
  }
  return null
}

async function clipboardText(): Promise<string> {
  try {
    return (await clipboard.readText()).trim().slice(0, MAX_COPIED_CHARS)
  } catch {
    return ''
  }
}

/**
 * What's on the clipboard, to offer: text, or an image as a small preview.
 * Text wins: a file copied in Finder also puts its icon there.
 */
export async function readClipboard(): Promise<
  { kind: 'text'; text: string } | { kind: 'image'; thumb: string } | undefined
> {
  const text = await clipboardText()
  if (text) return { kind: 'text', text }
  const image = await clipboardImage()
  return image ? { kind: 'image', thumb: thumbOf(image) } : undefined
}

/** The clipboard, attached: text as it is, an image saved as a file Claude can read. */
export async function takeClipboard(): Promise<CaptureAttachment | null> {
  const text = await clipboardText()
  if (text) return { kind: 'text', text }
  const image = await clipboardImage()
  if (!image) return null
  const file = newShotPath()
  fs.writeFileSync(file, image.toPNG())
  return { kind: 'image', path: file, thumb: thumbOf(image), copied: true }
}

/** Clears away old screenshots and copied images. */
export function pruneShots(now = Date.now()) {
  try {
    const dir = shotsDir()
    for (const name of fs.readdirSync(dir)) {
      const file = path.join(dir, name)
      if (now - fs.statSync(file).mtimeMs > KEEP_MS) fs.rmSync(file, { force: true })
    }
  } catch {
    // Nothing to clear.
  }
}
