// The floating capture panel: its own window, captured at its own size.
import { PNG } from 'pngjs'
import { projectId, sessionId, sleep, storefront, stub } from './helpers.mjs'

const area = 'Floating capture'

const FIELD = 'textarea.cap-field'

/**
 * Opens the panel through File › Capture…, as the user's double tap would,
 * without reading anything real: the board window counts as the app in front
 * (so no other app or browser tab is read) and the clipboard reads empty.
 * The panel's window then ignores the real pointer and stays up when the
 * user clicks elsewhere. Returns its page.
 */
async function openPanel(app) {
  await app.page.waitForSelector('.pv-item')
  await app.electron.evaluate(({ BrowserWindow, clipboard }) => {
    const board = BrowserWindow.getAllWindows().find((w) => !w.webContents.getURL().includes('#capture'))
    BrowserWindow.getFocusedWindow = () => board
    clipboard.readText = async () => ''
    clipboard.read = async () => []
  })
  await app.menu('File', 'Capture…')
  let panel
  for (let i = 0; i < 100 && !panel; i++) {
    panel = app.electron.windows().find((w) => w.url().includes('#capture'))
    if (!panel) await sleep(100)
  }
  await app.electron.evaluate(({ BrowserWindow }) => {
    const win = BrowserWindow.getAllWindows().find((w) => w.webContents.getURL().includes('#capture'))
    win.removeAllListeners('blur')
    win.setIgnoreMouseEvents(true)
    win.webContents.setBackgroundThrottling(false)
  })
  await panel.waitForSelector('.cap-card')
  await panel.waitForSelector('.cap-route:not([disabled])')
  await panel.evaluate(() => document.fonts.ready)
  return panel
}

/** Tells the panel more about what it opened over, as main does once it has read it. */
async function context(app, ctx) {
  await app.electron.evaluate(({ BrowserWindow }, ctx) => {
    BrowserWindow.getAllWindows()
      .find((w) => w.webContents.getURL().includes('#capture'))
      .webContents.send('ev:capture-context', ctx)
  }, ctx)
}

/** A made-up screenshot thumbnail: a browser-ish page of grey blocks. */
function thumb() {
  const w = 360
  const h = 225
  const png = new PNG({ width: w, height: h })
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4
      let c = [246, 246, 244]
      if (y < 22) c = [228, 228, 226]
      else if (y > 40 && y < 70 && x > 20 && x < 220) c = [60, 60, 64]
      else if (y > 90 && y < 190 && x > 20 && x < 160) c = [120, 170, 240]
      else if (y > 90 && y < 190 && x > 180 && x < 340) c = [250, 200, 120]
      png.data.set([...c, 255], i)
    }
  }
  return `data:image/png;base64,${PNG.sync.write(png).toString('base64')}`
}

/** The panel's shot: the panel's own window, its see-through margin kept. */
const shot = (panel, extra = {}) => ({ page: panel, omitBackground: true, ...extra })

export default [
  {
    id: 'capture-empty',
    area,
    title: 'The capture panel, just opened, nothing captured before',
    specs: ['CAP-081', 'CAP-082', 'CAP-086', 'CAP-087', 'CAP-088', 'CAP-089', 'CAP-105', 'CAP-106'],
    steps: ['Seed no earlier captures.', 'Open File › Capture…'],
    seed: (state) => {
      state.captures = []
    },
    async run({ app }) {
      return shot(await openPanel(app))
    }
  },
  {
    id: 'capture-recents',
    area,
    title: 'The capture panel with recent captures and their live status',
    specs: ['CAP-118', 'CAP-150', 'CAP-152', 'CAP-153'],
    steps: ['Start storefront’s sessions (sf-tax waiting, sf-cart running).', 'Open File › Capture…'],
    async run({ app }) {
      await storefront(app, { open: false })
      const panel = await openPanel(app)
      await panel.waitForSelector('.cap-recents .cap-recent')
      await sleep(500)
      return shot(panel)
    }
  },
  {
    id: 'capture-guessing',
    area,
    title: 'Typing: Claude working out where it goes',
    specs: ['CAP-086', 'CAP-088'],
    steps: ['Seed no earlier captures; the stand-in is slow to guess.', 'Open the panel and type a request.'],
    seed: (state) => {
      state.captures = []
    },
    responses: { delayMs: 120_000 },
    async run({ app }) {
      const panel = await openPanel(app)
      await panel.click(FIELD)
      await panel.keyboard.type('Fix the coupon field so it trims spaces before checking the code')
      await panel.waitForSelector('.cap-thinking')
      return shot(panel)
    }
  },
  {
    id: 'capture-guess',
    area,
    title: 'Typing, with Claude’s guess: a new session in storefront',
    specs: ['CAP-080', 'CAP-086', 'CAP-088', 'CAP-106'],
    steps: ['Seed no earlier captures.', 'Open the panel and type a request.', 'Wait for Claude’s guess (the stand-in answers: new session, storefront).'],
    seed: (state) => {
      state.captures = []
    },
    async run({ app }) {
      const panel = await openPanel(app)
      await panel.click(FIELD)
      await panel.keyboard.type('Fix the coupon field so it trims spaces before checking the code')
      await panel.waitForSelector('.cap-head .cap-who:has-text("Claude\'s guess")')
      return shot(panel)
    }
  },
  {
    id: 'capture-kind-menu',
    area,
    title: 'Where it goes: the kind and project menu',
    specs: ['CAP-107', 'CAP-108', 'CAP-109', 'CAP-110', 'CAP-112'],
    steps: ['As capture-guess.', 'Click the header (where it goes).'],
    seed: (state) => {
      state.captures = []
    },
    async run({ app }) {
      const panel = await openPanel(app)
      await panel.click(FIELD)
      await panel.keyboard.type('Fix the coupon field so it trims spaces before checking the code')
      await panel.waitForSelector('.cap-head .cap-who:has-text("Claude\'s guess")')
      await panel.click('.cap-route')
      await panel.waitForSelector('.cap-menu')
      await sleep(400)
      return shot(panel)
    }
  },
  {
    id: 'capture-attachments',
    area,
    title: 'Attachments: the browser tab, a screenshot, a file, and copied text',
    specs: ['CAP-091', 'CAP-092', 'CAP-093', 'CAP-094', 'CAP-095', 'CAP-098'],
    steps: [
      'Seed no earlier captures. Stand in for the screenshot, the file picker and the clipboard.',
      'Open the panel; it learns the browser tab in front and offers the copied text.',
      'Take a screenshot, attach a file, take the copied text, type a request.'
    ],
    seed: (state) => {
      state.captures = []
    },
    async run({ app }) {
      const copied = 'TypeError: Cannot read properties of undefined (reading ‘trim’) at applyCoupon (coupon.ts:42)'
      await stub(app, 'capture:screenshot', { value: { kind: 'image', path: '/tmp/nodus-parity-shot.png', thumb: thumb() } })
      await stub(app, 'capture:files', { value: [{ kind: 'file', path: '/tmp/coupon-spec.pdf', name: 'coupon-spec.pdf', size: 248_000 }] })
      await stub(app, 'capture:clipboard', { value: { kind: 'text', text: copied } })
      const panel = await openPanel(app)
      await context(app, { tab: { kind: 'tab', url: 'https://shop.example.com/checkout?coupon=SPRING', title: 'Checkout – Example Shop', app: 'Safari' } })
      await context(app, { clipboard: { kind: 'text', text: copied } })
      await panel.waitForSelector('.cap-tile-tab')
      await panel.click('button[aria-label="Take a screenshot"]')
      await panel.waitForSelector('.cap-tile-image')
      await panel.click('button[aria-label="Attach files"]')
      await panel.waitForSelector('.cap-tile-file')
      await panel.click('.cap-offer')
      await panel.waitForSelector('.cap-tile-text')
      await panel.click(FIELD)
      await panel.keyboard.type('The coupon field breaks on this page')
      await panel.waitForSelector('.cap-head .cap-who:has-text("Claude\'s guess")')
      await panel.mouse.move(2, 2)
      return shot(panel)
    }
  },
  {
    id: 'capture-receipt-note',
    area,
    title: 'The receipt after capturing a note',
    specs: ['CAP-140', 'CAP-141', 'CAP-146'],
    steps: ['Seed no earlier captures; the stand-in guesses a note in storefront.', 'Open the panel, type a reminder, wait for the guess and press ↩.'],
    seed: (state) => {
      state.captures = []
    },
    responses: { guess: { kind: 'note', project: projectId('storefront'), session: '', reason: 'It’s a reminder, not work to start now.' } },
    async run({ app }) {
      const panel = await openPanel(app)
      await panel.click(FIELD)
      await panel.keyboard.type('Remember to ask Sam about the coupon limits')
      await panel.waitForSelector('.cap-head .cap-who:has-text("Claude\'s guess")')
      await panel.keyboard.press('Enter')
      await panel.waitForSelector('.cap-receipt, .cap-receipt-note')
      // The receipt goes 2.6 s after it comes, so it's taken straight away.
      return shot(panel, { settleMs: 350, tries: 4 })
    }
  },
  {
    id: 'capture-receipt-session',
    area,
    title: 'The receipt after starting a session, with its sticker',
    specs: ['CAP-140', 'CAP-142', 'CAP-143', 'CAP-146'],
    steps: ['As capture-guess, with making the session stood in for (it would start Claude).', 'Press ↩.'],
    seed: (state) => {
      state.captures = []
    },
    async run({ app }) {
      await stub(app, 'capture:submit', {
        value: {
          kind: 'session',
          projectId: projectId('storefront'),
          projectName: 'storefront',
          title: 'Fix the coupon field so it trims spaces before checking the code',
          sessionId: sessionId('cap-new-session')
        }
      })
      const panel = await openPanel(app)
      await panel.click(FIELD)
      await panel.keyboard.type('Fix the coupon field so it trims spaces before checking the code')
      await panel.waitForSelector('.cap-head .cap-who:has-text("Claude\'s guess")')
      await panel.keyboard.press('Enter')
      await panel.waitForSelector('.cap-receipt')
      return shot(panel, { settleMs: 350, tries: 4 })
    }
  },
  {
    id: 'capture-permission',
    area,
    title: 'The panel asking for Accessibility, for the double tap',
    specs: ['CAP-032', 'CAP-117'],
    steps: ['Seed no earlier captures.', 'Open the panel and tell it the shortcut needs permission.'],
    seed: (state) => {
      state.captures = []
    },
    async run({ app }) {
      const panel = await openPanel(app)
      await context(app, { shortcut: 'needs-permission' })
      await panel.waitForSelector('.cap-banner')
      return shot(panel)
    }
  },
  {
    id: 'capture-screen-permission',
    area,
    title: 'A screenshot macOS refused: the Screen Recording notice',
    specs: ['CAP-100', 'CAP-101'],
    steps: ['Seed no earlier captures; the screenshot reports that Screen Recording isn’t allowed.', 'Open the panel and click Take a screenshot.'],
    seed: (state) => {
      state.captures = []
    },
    async run({ app }) {
      await stub(app, 'capture:screenshot', { value: 'needs-permission' })
      const panel = await openPanel(app)
      await panel.click('button[aria-label="Take a screenshot"]')
      await panel.waitForSelector('.cap-notice')
      await panel.mouse.move(2, 2)
      return shot(panel)
    }
  }
]
