// Views, actions, git, terminals, transcript preview, agent viewer.
import { card, node, rest, sleep, storefront, viewId } from './helpers.mjs'

const views = 'Views'
const actions = 'Actions rail'
const git = 'Git'
const terminals = 'Terminal modal, transcript preview, agent viewer'

const view = (key) => node(viewId(key))

/** Waits until a view's page (in its own frame) has drawn `selector`. */
async function pageDrawn(page, key, selector) {
  const prefix = `nodus-view://${viewId(key)}`
  for (let i = 0; i < 100; i++) {
    const frame = page.frames().find((f) => f.url().startsWith(prefix))
    if (frame) {
      await frame.waitForSelector(selector)
      return
    }
    await sleep(100)
  }
  throw new Error(`No page for view ${key}`)
}

/**
 * docs-site's board: Docs build status (a live page with data), Broken links
 * report (its brain building it) and Search index size (a command waiting
 * to be allowed), with a note of every colour.
 */
async function docsSite(app, { brain = 'working' } = {}) {
  await app.page.waitForSelector('.pv-item')
  await app.liven('docs-site', ['ds-brain-links'])
  await app.status('ds-brain-links', brain)
  await app.openProject('docs-site')
  await pageDrawn(app.page, 'ds-build', '#rows tr')
  await pageDrawn(app.page, 'ds-search-size', '#fill')
}

/** Shows the actions rail: the pointer at the window's right edge, then on the rail. */
async function actionsRail(page) {
  await page.mouse.move(1438, 450)
  await page.waitForSelector('.rail.is-right.is-open')
}

export default [
  // ---------------------------------------------------------------- views
  {
    id: 'views-board',
    area: views,
    title: 'docs-site’s board: a live view, a view being built, a command waiting for approval, notes of every colour',
    specs: ['VIEW-122', 'VIEW-124', 'VIEW-125', 'VIEW-126', 'VIEW-135', 'VIEW-137', 'VIEW-027'],
    steps: ['Start the Broken links report’s brain and report it working.', 'Open docs-site.', 'Wait for the two pages to draw.'],
    async run({ app, page }) {
      await docsSite(app)
      await rest(page)
    }
  },
  {
    id: 'views-brain-waiting',
    area: views,
    title: 'A view whose brain needs the user',
    specs: ['VIEW-124', 'VIEW-125', 'VIEW-137'],
    steps: ['As views-board, with the brain waiting on a permission.'],
    async run({ app, page }) {
      await docsSite(app, { brain: 'waiting' })
      await rest(page)
    }
  },
  {
    id: 'views-menu',
    area: views,
    title: 'A view’s menu (⋯)',
    specs: ['VIEW-134', 'VIEW-143'],
    steps: ['As views-board.', 'Point at Docs build status and click its More button.'],
    async run({ app, page }) {
      await docsSite(app)
      await page.hover(view('ds-build'))
      await page.click(`${view('ds-build')} button[aria-label=More]`)
      await page.waitForSelector('.ctx-menu')
    }
  },
  {
    id: 'views-large',
    area: views,
    title: 'A view opened large',
    specs: ['VIEW-133', 'VIEW-147'],
    steps: ['As views-board.', 'Point at Docs build status and click Open large.'],
    async run({ app, page }) {
      await docsSite(app)
      await page.hover(view('ds-build'))
      await page.click(`${view('ds-build')} button[aria-label="Open large"]`)
      await page.waitForSelector('.view-modal')
      await sleep(500)
      await rest(page)
    }
  },
  {
    id: 'views-change',
    area: views,
    title: 'Asking Claude to change a view (✎)',
    specs: ['VIEW-130', 'VIEW-131'],
    steps: ['As views-board.', 'Point at Docs build status, click Change view and type a request.'],
    async run({ app, page }) {
      await docsSite(app)
      await page.hover(view('ds-build'))
      await page.click(`${view('ds-build')} button[aria-label="Change view"]`)
      await page.waitForSelector('.view-ask-input')
      await page.keyboard.type('Show the failed build’s error')
      await rest(page)
    }
  },
  {
    id: 'views-new',
    area: views,
    title: 'The New view dialog',
    specs: ['VIEW-151', 'VIEW-152', 'VIEW-153', 'BOARD-144'],
    steps: ['As views-board.', 'Right-click the empty board and choose New view here…', 'Type a description.'],
    async run({ app, page }) {
      await docsSite(app)
      await page.mouse.click(1100, 720, { button: 'right' })
      await page.click('.ctx-item:has-text("New view here")')
      await page.waitForSelector('.modal-panel textarea')
      await page.keyboard.type('Open issues labelled docs, grouped by page')
      await rest(page)
    }
  },

  // ---------------------------------------------------------------- actions
  {
    id: 'actions-rail',
    area: actions,
    title: 'The actions rail slid out, an action pointed at',
    specs: ['ACT-046', 'ACT-047', 'ACT-049', 'ACT-054', 'ACT-055', 'ACT-058'],
    steps: ['As board-populated.', 'Move the pointer to the window’s right edge, then onto Open a PR.'],
    async run({ app, page }) {
      await storefront(app)
      await actionsRail(page)
      await page.hover('.rail.is-right button.rail-item[aria-label="Open a PR"]')
      await page.waitForSelector('.rail-hint.is-right')
    }
  },
  {
    id: 'actions-run',
    area: actions,
    title: 'An action running, fanned out beside the rail',
    specs: ['ACT-056', 'ACT-057', 'ACT-063', 'ACT-064', 'ACT-065'],
    steps: [
      'As board-populated.',
      'Slide the actions rail out and click Run tests (its run starts on the action’s prompt, so it reads as working).',
      'Move the pointer away until the rail hides, then slide it out again and point at the running action.'
    ],
    async run({ app, page }) {
      await storefront(app)
      await actionsRail(page)
      await page.click('.rail.is-right button.rail-item[aria-label="Run tests"]')
      await page.waitForSelector('.rail.is-right .rail-item.is-running')
      await rest(page)
      await page.waitForSelector('.rail.is-right:not(.is-open)')
      await actionsRail(page)
      await page.hover('.rail.is-right .rail-item.is-running')
      await page.waitForSelector('.action-fan .fan-card')
      await sleep(600)
    }
  },
  {
    id: 'actions-new',
    area: actions,
    title: 'New action: describing it',
    specs: ['ACT-061', 'ACT-067', 'ACT-068', 'ACT-069', 'ACT-070', 'ACT-071'],
    steps: ['As board-populated.', 'Slide the actions rail out and click +.', 'Type what the button should do.'],
    async run({ app, page }) {
      await storefront(app)
      await actionsRail(page)
      await page.click('.rail.is-right .rail-add')
      await page.waitForSelector('.action-description')
      await page.keyboard.type('Check the bundle size of each route against main')
      await rest(page)
    }
  },
  {
    id: 'actions-drafting',
    area: actions,
    title: 'New action: Claude drafting it',
    specs: ['ACT-072'],
    steps: ['As actions-new, with the stand-in slow to answer.', 'Click Draft with Claude.'],
    responses: { delayMs: 120_000 },
    async run({ app, page }) {
      await storefront(app)
      await actionsRail(page)
      await page.click('.rail.is-right .rail-add')
      await page.waitForSelector('.action-description')
      await page.keyboard.type('Check the bundle size of each route against main')
      await page.click('.action-dialog button:has-text("Draft with Claude")')
      await page.waitForSelector('.action-dialog button:has-text("Drafting")')
      await rest(page)
    }
  },
  {
    id: 'actions-review',
    area: actions,
    title: 'New action: reviewing Claude’s draft',
    specs: ['ACT-074', 'ACT-076', 'ACT-077'],
    steps: ['As actions-drafting, with the stand-in answering at once (Check bundle size, gauge icon).'],
    async run({ app, page }) {
      await storefront(app)
      await actionsRail(page)
      await page.click('.rail.is-right .rail-add')
      await page.waitForSelector('.action-description')
      await page.keyboard.type('Check the bundle size of each route against main')
      await page.click('.action-dialog button:has-text("Draft with Claude")')
      await page.waitForSelector('.action-name')
      await rest(page)
    }
  },

  // ---------------------------------------------------------------- git
  {
    id: 'git-log',
    area: git,
    title: 'The branch chip pointed at: tracking and the last five commits',
    specs: ['GIT-026', 'GIT-027', 'GIT-029', 'GIT-030', 'GIT-031'],
    steps: ['As board-populated.', 'Point at the branch chip in the title bar.'],
    async run({ app, page }) {
      await storefront(app)
      await page.hover('.git-branch')
      await page.waitForSelector('.git-log-commit')
    }
  },
  {
    id: 'git-diff',
    area: git,
    title: 'The Changes window: modified, added, deleted and renamed files, a diff shown',
    specs: ['GIT-025', 'GIT-032', 'GIT-034', 'GIT-036', 'GIT-037', 'GIT-040'],
    steps: ['As board-populated.', 'Click Changes in the title bar.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click('.git-changes')
      await page.waitForSelector('.diff-modal')
      await sleep(800)
      await rest(page)
    }
  },

  // ---------------------------------------------------------------- terminals
  {
    id: 'terminal-claude',
    area: terminals,
    title: 'A Claude session’s terminal window',
    specs: ['MODAL-001', 'MODAL-014', 'MODAL-019', 'MODAL-021', 'TERM-002'],
    steps: ['As board-populated.', 'Click sf-cart’s card.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(card('sf-cart'))
      await page.waitForSelector('.term-modal .xterm canvas')
      await sleep(1200)
      await rest(page)
    }
  },
  {
    id: 'terminal-shell',
    area: terminals,
    title: 'A terminal card’s window',
    specs: ['MODAL-012', 'MODAL-015'],
    steps: ['As board-populated.', 'Click the terminal card.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(card('sf-terminal'))
      await page.waitForSelector('.term-modal .xterm canvas')
      await sleep(1200)
      await rest(page)
    }
  },
  {
    id: 'terminal-stop-confirm',
    area: terminals,
    title: 'Stopping a working session asks first',
    specs: ['MODAL-022', 'MODAL-023'],
    steps: ['As board-populated.', 'Click sf-skeleton’s card (working).', 'Click Stop in its window.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(card('sf-skeleton'))
      await page.waitForSelector('.term-modal .xterm canvas')
      await page.click('.term-modal .term-actions button:has-text("Stop")')
      await page.waitForSelector('.confirm-dialog')
      await rest(page)
    }
  },
  {
    id: 'transcript-preview',
    area: terminals,
    title: 'An archived session’s transcript',
    specs: ['TRANS-001', 'TRANS-007'],
    steps: ['As board-populated.', 'Right-click the archive stack’s top card and choose Preview.'],
    async run({ app, page }) {
      await storefront(app)
      await page.mouse.click(60 + 172 * 0.8, 148 + 200 * 0.8, { button: 'right' })
      await page.click('.ctx-item:has-text("Preview")')
      await page.waitForSelector('.tp-modal .tp-msg-content')
      await rest(page)
    }
  },
  {
    id: 'agent-viewer',
    area: terminals,
    title: 'The agent viewer: a session’s subagents, the first in the fan shown',
    specs: ['AGV-001', 'AGV-005', 'AGV-006', 'AGV-009'],
    steps: ['As board-deck-fanned.', 'Click the first agent in the fan.'],
    async run({ app, page }) {
      await storefront(app)
      await page.mouse.move(60 + 544 * 0.8, 148 + 263 * 0.8)
      await page.waitForSelector('.agent-fan .fan-card')
      await page.click('.agent-fan .fan-card >> nth=0')
      await page.waitForSelector('.av-modal .av-item')
      await sleep(400)
      await rest(page)
    }
  }
]

