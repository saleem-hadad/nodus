// The board: canvas, cards, sections, archive, notes, agent deck, expanded cards, menus, zoom.
import { rest, storefront } from './helpers.mjs'

const area = 'Board'

export default [
  {
    id: 'board-populated',
    area,
    title: 'storefront’s board: cards in every status, Checkout section, lineage edges, sticker, signed note, ink, archive, note pad',
    specs: [],
    steps: ['Start storefront’s sessions (as in projects-grid).', 'Open storefront from the grid (double-click).'],
    async run({ app, page }) {
      await storefront(app)
      await rest(page)
    }
  }
]
