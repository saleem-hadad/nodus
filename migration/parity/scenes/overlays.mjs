// Prompt bar, ⌘K search, merge dialog, ink.
import { card, rest, sleep, storefront } from './helpers.mjs'

const area = 'Prompt bar, ⌘K search, merge dialog, ink'

/** storefront opens at x 60, y 96, zoom 0.8, under the 52px title bar: flow → page coordinates. */
const at = (x, y) => ({ x: 60 + x * 0.8, y: 52 + 96 + y * 0.8 })

const FIELD = '.sidekick-field'

/**
 * Replaces the summarize handler so the merge dialog's progress is set, not
 * run: each session gets `phases[i]`; with `results`, it resolves with them,
 * otherwise it stays pending.
 */
async function summaries(app, { phases, results }) {
  await app.electron.evaluate(
    ({ ipcMain }, { phases, results }) => {
      ipcMain.removeHandler('session:summarize')
      ipcMain.handle('session:summarize', (event, _projectId, ids, requestId) => {
        ids.forEach((sessionId, i) => {
          const phase = phases?.[i]
          if (phase) event.sender.send('ev:summarize-progress', { requestId, sessionId, phase, ...(phase === 'error' ? { error: 'claude exited with code 1' } : {}) })
        })
        if (!results) return new Promise(() => {})
        return ids.map((sessionId, i) => ({ sessionId, title: '', ...results[i] }))
      })
    },
    { phases, results }
  )
}

const MERGE = ['sf-cart', 'sf-coupons', 'sf-search-plan']

async function startMerge(app, page) {
  await storefront(app)
  for (const key of MERGE) await page.click(card(key), { modifiers: ['Shift'] })
  await page.click('.selection-merge')
  await page.waitForSelector('.merge-inner')
}

/** Picks an ink tool from the dock. */
const tool = (page, name) => page.click(`.ink-dock button[aria-label="${name}"]`)

async function stroke(page, points) {
  await page.mouse.move(points[0].x, points[0].y)
  await page.mouse.down()
  for (const p of points.slice(1)) await page.mouse.move(p.x, p.y, { steps: 6 })
  await page.mouse.up()
}

export default [
  {
    id: 'bar-focused',
    area,
    title: 'The prompt bar, focused and empty',
    specs: ['BAR-004', 'BAR-006', 'BAR-007'],
    steps: ['As board-populated.', 'Click the prompt bar.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(FIELD)
      await rest(page)
    }
  },
  {
    id: 'bar-typing',
    area,
    title: 'Typing a multi-line prompt',
    specs: ['BAR-005', 'BAR-009'],
    steps: ['As board-populated.', 'Click the prompt bar and type three lines (⇧↩ between them).'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(FIELD)
      await page.keyboard.type('Add a sort menu to the search results:')
      await page.keyboard.press('Shift+Enter')
      await page.keyboard.type('- price, low to high and high to low')
      await page.keyboard.press('Shift+Enter')
      await page.keyboard.type('- newest first')
      await rest(page)
    }
  },
  {
    id: 'bar-slash-menu',
    area,
    title: 'Typing / in the prompt bar: the menu of what else it makes',
    specs: ['BAR-014', 'BAR-015'],
    steps: ['As board-populated.', 'Press / on the board.'],
    async run({ app, page }) {
      await storefront(app)
      await rest(page)
      await page.keyboard.press('/')
      await page.waitForSelector('.sidekick-menu')
    }
  },
  {
    id: 'bar-view',
    area,
    title: 'The /view token, with a description typed',
    specs: ['BAR-006', 'BAR-017', 'BAR-019'],
    steps: ['As board-populated.', 'Click the prompt bar, type “/view ”, then a description.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(FIELD)
      await page.keyboard.type('/view ')
      await page.waitForSelector('.sidekick-token.is-view')
      await page.keyboard.type('failing tests by file')
      await rest(page)
    }
  },
  {
    id: 'bar-note',
    area,
    title: 'The /note token, with its colour swatch',
    specs: ['BAR-017', 'BAR-019', 'BAR-022'],
    steps: ['As board-populated.', 'Click the prompt bar, type “/note ”, then the note.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(FIELD)
      await page.keyboard.type('/note ')
      await page.waitForSelector('.sidekick-token.is-note')
      await page.keyboard.type('Check the coupon edge cases with Sam')
      await rest(page)
    }
  },
  {
    id: 'bar-one-selected',
    area,
    title: 'One session selected: the bar sends into it',
    specs: ['BAR-020', 'BAR-024'],
    steps: ['As board-populated.', '⇧-click sf-cart (running, done).', 'Click the prompt bar.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(card('sf-cart'), { modifiers: ['Shift'] })
      await page.click(FIELD)
      await rest(page)
    }
  },
  {
    id: 'bar-several-selected',
    area,
    title: 'Several sessions selected: the bar asks about them',
    specs: ['BAR-006', 'BAR-025', 'SEL-001'],
    steps: ['As board-populated.', '⇧-click sf-coupons and sf-search-plan.', 'Click the prompt bar and type a question.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(card('sf-coupons'), { modifiers: ['Shift'] })
      await page.click(card('sf-search-plan'), { modifiers: ['Shift'] })
      await page.click(FIELD)
      await page.keyboard.type('What do these two have in common?')
      await rest(page)
    }
  },
  {
    id: 'search-empty',
    area,
    title: '⌘K search, just opened: the latest sessions',
    specs: ['SRCH-001', 'SRCH-005', 'SRCH-010'],
    steps: ['As board-populated.', 'Press ⌘K.'],
    async run({ app, page }) {
      await storefront(app)
      await rest(page)
      await page.keyboard.press('Meta+k')
      await page.waitForSelector('.search-dialog .search-row-body')
    }
  },
  {
    id: 'search-results',
    area,
    title: '⌘K search results with highlighted snippets',
    specs: ['SRCH-006', 'SRCH-010', 'SRCH-011'],
    steps: ['As board-populated.', 'Press ⌘K and type “coupon”.'],
    async run({ app, page }) {
      await storefront(app)
      await rest(page)
      await page.keyboard.press('Meta+k')
      await page.waitForSelector('.search-input')
      await page.keyboard.type('coupon')
      await page.waitForSelector('.search-snippet')
      await sleep(400)
    }
  },
  {
    id: 'search-none',
    area,
    title: '⌘K search with no match',
    specs: ['SRCH-008'],
    steps: ['As board-populated.', 'Press ⌘K and type “zebra”.'],
    async run({ app, page }) {
      await storefront(app)
      await rest(page)
      await page.keyboard.press('Meta+k')
      await page.waitForSelector('.search-input')
      await page.keyboard.type('zebra')
      await page.waitForSelector('.search-empty')
      await sleep(400)
    }
  },
  {
    id: 'merge-progress',
    area,
    title: 'The merge dialog summarizing: one ready, one running, one queued',
    specs: ['MERGE-001', 'MERGE-004', 'MERGE-005'],
    steps: [
      'As board-populated, with summaries reported as they go and never finishing.',
      '⇧-click sf-cart, sf-coupons and sf-search-plan; click New session from context.'
    ],
    async run({ app, page }) {
      await summaries(app, { phases: ['done', 'running', 'queued'] })
      await startMerge(app, page)
      await page.waitForSelector('.merge-progress-row[data-phase=running]')
      await rest(page)
    }
  },
  {
    id: 'merge-failed',
    area,
    title: 'The merge dialog with a summary that failed: Retry and Skip',
    specs: ['MERGE-007', 'MERGE-009', 'MERGE-010'],
    steps: ['As merge-progress, with sf-coupons’ summary failing and the others ready.'],
    async run({ app, page }) {
      await summaries(app, {
        phases: ['done', 'error', 'done'],
        results: [
          { summary: 'Moved the cart to a Zustand store; useCart and useCartCount read it with selectors.' },
          { summary: '', error: 'claude exited with code 1' },
          { summary: 'Planned the v2 search page: skeleton tiles, a filter sheet, and an empty state.' }
        ]
      })
      await startMerge(app, page)
      await page.waitForSelector('.merge-review')
      await rest(page)
    }
  },
  {
    id: 'merge-ready',
    area,
    title: 'The merge dialog ready: the summaries to start from',
    specs: ['MERGE-007', 'MERGE-008', 'MERGE-011'],
    steps: ['As merge-progress, with every summary ready.'],
    async run({ app, page }) {
      await summaries(app, {
        phases: ['done', 'done', 'done'],
        results: [
          { summary: 'Moved the cart to a Zustand store; useCart and useCartCount read it with selectors.' },
          { summary: 'Wrote Playwright tests for coupon codes; the sale-only case fails on purpose.' },
          { summary: 'Planned the v2 search page: skeleton tiles, a filter sheet, and an empty state.' }
        ]
      })
      await startMerge(app, page)
      await page.waitForSelector('.merge-review')
      await rest(page)
    }
  },
  {
    id: 'ink-pen',
    area,
    title: 'Drawing with the pen (purple)',
    specs: ['INKUI-001', 'INKUI-002', 'INKUI-004', 'INKUI-008', 'INKUI-010'],
    steps: ['As board-populated.', 'Pick the Pen, then Purple from the colour button.', 'Draw a stroke on the empty board below the terminal card.'],
    async run({ app, page }) {
      await storefront(app)
      await tool(page, 'Pen')
      await page.click('.ink-color button.ink-tool')
      await page.click('.ink-palette button[aria-label="Purple"]')
      const a = at(1080, 650)
      await stroke(page, [a, { x: a.x + 60, y: a.y + 30 }, { x: a.x + 140, y: a.y - 10 }, { x: a.x + 220, y: a.y + 40 }])
      await rest(page)
    }
  },
  {
    id: 'ink-palette',
    area,
    title: 'The ink dock with its colour palette open',
    specs: ['INKUI-001', 'INKUI-004'],
    steps: ['As board-populated.', 'Click the ink colour button.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click('.ink-color button.ink-tool')
      await page.waitForSelector('.ink-palette')
      await rest(page)
    }
  },
  {
    id: 'ink-arrow',
    area,
    title: 'Drawing an arrow',
    specs: ['INKUI-009'],
    steps: ['As board-populated.', 'Pick the Arrow (A).', 'Drag from below the terminal card up towards it.'],
    async run({ app, page }) {
      await storefront(app)
      await tool(page, 'Arrow')
      const a = at(1300, 700)
      await stroke(page, [a, at(1230, 610)])
      await rest(page)
    }
  },
  {
    id: 'ink-text',
    area,
    title: 'Writing an ink label',
    specs: ['INKUI-013'],
    steps: ['As board-populated.', 'Pick Text.', 'Click the empty board below the terminal card and type “needs tests”.'],
    async run({ app, page }) {
      await storefront(app)
      await tool(page, 'Text')
      const a = at(1100, 660)
      await page.mouse.click(a.x, a.y)
      await page.waitForSelector('.ink-label-input')
      await page.keyboard.type('needs tests')
      await rest(page)
    }
  },
  {
    id: 'ink-eraser',
    area,
    title: 'The eraser over the wavy stroke, mid-erase',
    specs: ['INKUI-011'],
    steps: ['As board-populated.', 'Pick the Eraser (X).', 'Press on the wavy graphite stroke and drag along it, without letting go.'],
    async run({ app, page }) {
      await storefront(app)
      await tool(page, 'Eraser')
      const a = at(430, 700)
      await page.mouse.move(a.x, a.y)
      await page.mouse.down()
      await page.mouse.move(at(520, 700).x, at(520, 700).y, { steps: 8 })
      await page.waitForSelector('.is-erasing')
    }
  },
  {
    id: 'ink-selected',
    area,
    title: 'A selected ink stroke',
    specs: ['INKUI-015', 'INKUI-026'],
    steps: ['As board-populated.', 'With the pointer tool, click the green “ship Friday” label.'],
    async run({ app, page }) {
      await storefront(app)
      const a = at(470, 652)
      await page.mouse.click(a.x, a.y)
      await page.waitForSelector('.ink-selection')
      await rest(page)
    }
  }
]

