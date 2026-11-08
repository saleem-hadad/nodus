// The board: canvas, cards, sections, archive, notes, agent deck, expanded cards, menus, zoom.
import { withExtra } from '../seed/index.mjs'
import { board, card, node, noteId, rest, sectionId, sleep, storefront, viewport } from './helpers.mjs'

const area = 'Board'

/** storefront opens at x 60, y 96, zoom 0.8, under the 52px title bar: flow → page coordinates. */
const at = (x, y) => ({ x: 60 + x * 0.8, y: 52 + 96 + y * 0.8 })
/** The archive stack's top card. */
const STACK = at(172, 200)
/** Empty board below the terminal card. */
const EMPTY = at(1150, 690)

async function rightClick(page, { x, y }) {
  await page.mouse.click(x, y, { button: 'right' })
  await page.waitForSelector('.ctx-menu')
}

/** Waits for an expanded card's terminal to be drawn by its stand-in. */
async function terminalDrawn(page, sel) {
  await page.waitForSelector(`${sel} .xterm canvas`)
  await sleep(1200)
}

export default [
  {
    id: 'board-empty',
    area,
    title: 'An empty board: no sessions, the hint, an empty archive and the note pad',
    specs: ['BOARD-007', 'BOARD-146', 'ARCH-001', 'NOTEUI-040', 'NOTEUI-042'],
    steps: ['Open mobile-app (it has no sessions).'],
    async run({ app, page }) {
      await board(app, 'mobile-app')
      await page.waitForSelector('.board-hint')
      await rest(page)
    }
  },
  {
    id: 'board-populated',
    area,
    title: 'storefront’s board: cards in every status, Checkout section, lineage edges, agent deck, sticker, signed note, ink, archive, note pad',
    specs: ['CARD-001', 'CARD-004', 'CARD-011', 'CARD-012', 'CARD-013', 'CARD-014', 'CARD-016', 'CARD-019', 'CARD-025', 'DECK-002', 'DECK-004', 'BOARD-010', 'BOARD-020', 'NOTEUI-019', 'INKUI-025'],
    steps: [
      'Start storefront’s sessions: sf-skeleton working (two agents working, one done), sf-tax waiting, sf-cart done but running, the terminal card’s shell.',
      'Open storefront from the grid (double-click).'
    ],
    async run({ app, page }) {
      await storefront(app)
      await rest(page)
    }
  },
  {
    id: 'board-card-hover',
    area,
    title: 'A card pointed at: its Archive and Expand buttons',
    specs: ['CARD-003', 'CARD-020', 'CARD-021'],
    steps: ['As board-populated.', 'Point at sf-cart’s card.'],
    async run({ app, page }) {
      await storefront(app)
      await page.hover(card('sf-cart'))
      await sleep(200)
    }
  },
  {
    id: 'board-selected',
    area,
    title: 'Two cards selected, with the selection bar',
    specs: ['CARD-005', 'BOARD-040', 'SEL-001', 'SEL-002'],
    steps: ['As board-populated.', '⇧-click sf-cart and sf-coupons.', 'Move the pointer away.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(card('sf-cart'), { modifiers: ['Shift'] })
      await page.click(card('sf-coupons'), { modifiers: ['Shift'] })
      await page.waitForSelector('.selection-bar')
      await rest(page)
    }
  },
  {
    id: 'board-marquee',
    area,
    title: 'Dragging a selection box over the board',
    specs: ['BOARD-040', 'CARD-005'],
    steps: ['As board-populated.', 'Press on the empty board below the terminal card and drag up and left over the Plan card, without letting go.'],
    async run({ app, page }) {
      await storefront(app)
      await page.mouse.move(EMPTY.x, EMPTY.y)
      await page.mouse.down()
      await page.mouse.move(EMPTY.x - 150, EMPTY.y - 100, { steps: 4 })
      await page.mouse.move(EMPTY.x - 330, EMPTY.y - 260, { steps: 4 })
      await page.waitForSelector('.react-flow__selection')
    }
  },
  {
    id: 'board-section-naming',
    area,
    title: 'A new section while Claude names it',
    specs: ['BOARD-086', 'BOARD-087', 'BOARD-089', 'BOARD-162'],
    steps: ['As board-populated, with the stand-in slow to name sections.', '⇧-click sf-coupons and sf-search-plan.', 'Press ⌘G.'],
    responses: { delayMs: 120_000 },
    async run({ app, page }) {
      await storefront(app)
      await page.click(card('sf-coupons'), { modifiers: ['Shift'] })
      await page.click(card('sf-search-plan'), { modifiers: ['Shift'] })
      await page.keyboard.press('Meta+g')
      await page.waitForSelector('.section-name.is-naming')
      await rest(page)
    }
  },
  {
    id: 'board-section-rename',
    area,
    title: 'Renaming a section',
    specs: ['BOARD-045', 'BOARD-093'],
    steps: ['As board-populated.', 'Double-click the Checkout section’s header.'],
    async run({ app, page }) {
      await storefront(app)
      await page.dblclick(`${node(sectionId('checkout'))} .section-label`)
      await page.waitForSelector('.section-name-input')
      await rest(page)
    }
  },
  {
    id: 'board-archive-hover',
    area,
    title: 'The archive stack pointed at: its search field',
    specs: ['ARCH-019', 'ARCH-020', 'ARCH-021'],
    steps: ['As board-populated.', 'Point at the archive stack’s top card.'],
    async run({ app, page }) {
      await storefront(app)
      await page.mouse.move(STACK.x, STACK.y)
      await page.waitForSelector('.archive-search input:focus')
    }
  },
  {
    id: 'board-archive-open',
    area,
    title: 'The archive fanned open',
    specs: ['ARCH-004', 'ARCH-011', 'ARCH-014'],
    steps: ['As board-populated.', 'Click the archive stack.', 'Move the pointer away.'],
    async run({ app, page }) {
      await storefront(app)
      await page.mouse.click(STACK.x, STACK.y)
      await sleep(800)
      await rest(page)
    }
  },
  {
    id: 'board-archive-search',
    area,
    title: 'Searching the archive: matches fanned out, matched letters marked',
    specs: ['ARCH-022', 'CARD-018'],
    steps: ['As board-populated.', 'Point at the archive stack.', 'Type “fix”.'],
    async run({ app, page }) {
      await storefront(app)
      await page.mouse.move(STACK.x, STACK.y)
      await page.waitForSelector('.archive-search input:focus')
      await page.keyboard.type('fix')
      await sleep(800)
    }
  },
  {
    id: 'board-note-editing',
    area,
    title: 'Writing in a note',
    specs: ['NOTEUI-010', 'NOTEUI-011'],
    steps: ['As board-populated.', 'Double-click the yellow note “Ask design about empty states…”.'],
    async run({ app, page }) {
      await storefront(app)
      await page.dblclick(node(noteId('sf-empty-states')))
      await page.waitForSelector('.note-input')
      await rest(page)
    }
  },
  {
    id: 'board-note-selected',
    area,
    title: 'A selected note with its resize handles',
    specs: ['NOTEUI-019', 'NOTEUI-021', 'NOTEUI-025'],
    steps: ['As board-populated.', 'Click the signed blue note.', 'Move the pointer away.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(node(noteId('sf-signed')))
      await page.waitForSelector('.note-resize-handle')
      await rest(page)
    }
  },
  {
    id: 'board-pad-hover',
    area,
    title: 'The note pad pointed at',
    specs: ['NOTEUI-043', 'NOTEUI-044'],
    steps: ['As board-populated.', 'Point at the pad’s top sheet.'],
    async run({ app, page }) {
      await storefront(app)
      // The pad's top sheet, under the archive stack.
      const pad = at(172, 546)
      await page.mouse.move(pad.x, pad.y)
      await sleep(200)
    }
  },
  {
    id: 'board-deck-fanned',
    area,
    title: 'The agent deck fanned out beside its card',
    specs: ['DECK-005', 'DECK-008', 'DECK-010'],
    steps: ['As board-populated.', 'Point at the agent deck under sf-skeleton’s card.'],
    async run({ app, page }) {
      await storefront(app)
      // The layers peek out under the card's bottom edge (flow y 256).
      const deck = at(544, 263)
      await page.mouse.move(deck.x, deck.y)
      await page.waitForSelector('.agent-fan')
      await sleep(400)
    }
  },
  {
    id: 'board-expanded-stopped',
    area,
    title: 'Expanded cards whose processes aren’t running: Resume session and Start shell',
    specs: ['XCARD-011', 'XCARD-022'],
    steps: ['Open infra: an expanded Claude card and an expanded terminal card, neither running.'],
    async run({ app, page }) {
      await board(app, 'infra')
      await page.waitForSelector(`${card('in-certs')} .term-overlay.is-blocking`)
      await rest(page)
    }
  },
  {
    id: 'board-expanded',
    area,
    title: 'Expanded cards running: a Claude session with its usage, and a terminal',
    specs: ['XCARD-003', 'XCARD-005', 'XCARD-007', 'XCARD-012', 'TERM-002'],
    steps: [
      'Open infra.',
      'Click Resume session on in-certs and Start shell on the terminal card.',
      'Report in-certs’ model, context and cost through its status line.'
    ],
    async run({ app, page }) {
      await board(app, 'infra')
      for (const key of ['in-certs', 'in-shell']) {
        await page.click(`${card(key)} .term-overlay.is-blocking .btn`)
        await terminalDrawn(page, card(key))
      }
      await app.usage('in-certs')
      await rest(page)
    }
  },
  {
    id: 'board-menu-pane',
    area,
    title: 'The board’s context menu',
    specs: ['BOARD-172', 'BOARD-173', 'BOARD-176', 'BOARD-177'],
    steps: ['As board-populated.', 'Right-click the empty board below the terminal card.'],
    async run({ app, page }) {
      await storefront(app)
      await rightClick(page, EMPTY)
    }
  },
  {
    id: 'board-menu-card',
    area,
    title: 'A card’s context menu (in a section)',
    specs: ['BOARD-172', 'BOARD-175', 'BOARD-087'],
    steps: ['As board-populated.', 'Right-click sf-cart’s card.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(card('sf-cart'), { button: 'right' })
      await page.waitForSelector('.ctx-menu')
    }
  },
  {
    id: 'board-menu-note',
    area,
    title: 'A note’s context menu',
    specs: ['BOARD-120', 'BOARD-175'],
    steps: ['As board-populated.', 'Right-click the yellow note.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(node(noteId('sf-empty-states')), { button: 'right' })
      await page.waitForSelector('.ctx-menu')
    }
  },
  {
    id: 'board-menu-section',
    area,
    title: 'A section’s context menu',
    specs: ['BOARD-096', 'BOARD-095'],
    steps: ['As board-populated.', 'Right-click the Checkout section’s header.'],
    async run({ app, page }) {
      await storefront(app)
      await page.click(`${node(sectionId('checkout'))} .section-label`, { button: 'right' })
      await page.waitForSelector('.ctx-menu')
    }
  },
  {
    id: 'board-menu-archived',
    area,
    title: 'An archived card’s context menu',
    specs: ['BOARD-175', 'BOARD-044'],
    steps: ['As board-populated.', 'Right-click the archive stack’s top card.'],
    async run({ app, page }) {
      await storefront(app)
      await rightClick(page, STACK)
    }
  },
  {
    id: 'board-confirm-archive',
    area,
    title: 'Archiving a working session asks first',
    specs: ['CARD-022', 'BOARD-110'],
    steps: ['As board-populated.', 'Point at sf-skeleton’s card and click its Archive button.'],
    async run({ app, page }) {
      await storefront(app)
      await page.hover(card('sf-skeleton'))
      await page.click(`${card('sf-skeleton')} .card-corner button[aria-label=Archive]`)
      await page.waitForSelector('.confirm-dialog')
      await rest(page)
    }
  },
  {
    id: 'board-toast',
    area,
    title: 'The board’s toast: an action run that finished while the board was closed',
    specs: ['BOARD-106', 'BAR-032'],
    steps: ['Seed the Run tests action run on storefront’s board, not running.', 'Open storefront.'],
    seed: withExtra('run'),
    async run({ app, page }) {
      await board(app, 'storefront')
      await page.waitForSelector('.board-toast')
      await rest(page)
    }
  },
  {
    id: 'board-zoomed-out',
    area,
    title: 'storefront’s board zoomed out to 40%',
    specs: ['BOARD-033'],
    steps: ['Seed storefront’s viewport at zoom 0.4.', 'As board-populated.'],
    seed: viewport('storefront', { x: 380, y: 230, zoom: 0.4 }),
    async run({ app, page }) {
      await storefront(app)
      await rest(page)
    }
  },
  {
    id: 'board-zoomed-in',
    area,
    title: 'storefront’s board zoomed in to 160%',
    specs: ['BOARD-033'],
    steps: ['Seed storefront’s viewport at zoom 1.6 over the Checkout section.', 'As board-populated.'],
    seed: viewport('storefront', { x: -1120, y: 40, zoom: 1.6 }),
    async run({ app, page }) {
      await storefront(app)
      await rest(page)
    }
  }
]
