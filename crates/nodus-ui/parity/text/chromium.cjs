// Draws page.html as 0.9.4's Chromium does (Electron 44.5.1 at a device scale
// of 2) and saves chromium.png and page.json (each case's layout rect), with no
// Dock icon and without ever taking the focus.
// usage: <electron> chromium.cjs <dir with page.html>
const { app, BrowserWindow } = require('electron')
const fs = require('fs')
const path = require('path')
app.commandLine.appendSwitch('force-device-scale-factor', '2')
if (process.platform === 'darwin') app.setActivationPolicy('prohibited')
const dir = path.resolve(process.argv[process.argv.length - 1])
app.whenReady().then(async () => {
  const win = new BrowserWindow({
    width: 1200,
    height: 760,
    useContentSize: true,
    frame: false,
    enableLargerThanScreen: true,
    show: false,
    focusable: false,
    paintWhenInitiallyHidden: true,
    webPreferences: { backgroundThrottling: false }
  })
  await win.loadFile(path.join(dir, 'page.html'))
  await win.webContents.executeJavaScript('document.fonts.ready.then(() => 1)')
  const rects = await win.webContents.executeJavaScript('window.measure()')
  fs.writeFileSync(path.join(dir, 'page.json'), JSON.stringify(rects, null, 1))
  await new Promise((resolve) => setTimeout(resolve, 300))
  const image = await win.webContents.capturePage()
  fs.writeFileSync(path.join(dir, 'chromium.png'), image.toPNG())
  app.exit(0)
})
