import { app, BrowserWindow, nativeTheme, shell } from 'electron'
import { join } from 'path'
import { pathToFileURL } from 'url'
import { registerIpc, shutdown } from './ipc'

let mainWindow: BrowserWindow | null = null

/** Matches --bg-window, so resizing or reloading never flashes the other theme. */
const windowBackground = () => (nativeTheme.shouldUseDarkColors ? '#1e1e20' : '#f5f5f4')

const rendererUrl =
  process.env.ELECTRON_RENDERER_URL ?? pathToFileURL(join(__dirname, '../renderer/index.html')).href

/** Whether a URL is the app's own page, the only one the window may show. */
function isRenderer(url: string) {
  try {
    const target = new URL(url)
    target.hash = ''
    return target.href === new URL(rendererUrl).href
  } catch {
    return false
  }
}

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1440,
    height: 920,
    minWidth: 960,
    minHeight: 620,
    show: false,
    title: 'Nodus',
    titleBarStyle: 'hiddenInset',
    trafficLightPosition: { x: 18, y: 18 },
    backgroundColor: windowBackground(),
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false
    }
  })

  mainWindow.once('ready-to-show', () => mainWindow?.show())

  // Links clicked inside terminals open in the default browser, never in-app.
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:/.test(url)) shell.openExternal(url)
    return { action: 'deny' }
  })

  // Any page shown here would get window.api, which runs commands, so the
  // window never leaves the app. Web links go to the browser instead.
  mainWindow.webContents.on('will-navigate', (event, url) => {
    if (isRenderer(url)) return
    event.preventDefault()
    if (/^https?:/.test(url)) shell.openExternal(url)
  })

  if (process.env.ELECTRON_RENDERER_URL) {
    mainWindow.loadURL(process.env.ELECTRON_RENDERER_URL)
  } else {
    mainWindow.loadFile(join(__dirname, '../renderer/index.html'))
  }

  mainWindow.on('closed', () => {
    mainWindow = null
  })
}

app.setName('Nodus')

// Keeps development or test state separate from the real app data.
if (process.env.NODUS_USER_DATA) app.setPath('userData', process.env.NODUS_USER_DATA)

app.whenReady().then(async () => {
  await registerIpc(() => mainWindow)
  createWindow()

  // Fires for the Settings theme and for macOS appearance changes alike.
  nativeTheme.on('updated', () => mainWindow?.setBackgroundColor(windowBackground()))

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

app.on('window-all-closed', () => {
  // The app stays open on macOS. Either way sessions keep running in the session host.
  if (process.platform !== 'darwin') app.quit()
})

app.on('before-quit', () => {
  shutdown()
})
