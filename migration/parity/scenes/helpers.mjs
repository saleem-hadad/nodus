// Steps several scenes share.
import { agentIdOf } from '../seed/index.mjs'
import { noteId, projectId, sectionId, sessionId, viewId } from '../seed/world.mjs'
import { sleep } from '../lib/app.mjs'

export { agentIdOf, noteId, projectId, sectionId, sessionId, viewId, sleep }

/** A board node by the id of what it draws. */
export const node = (id) => `.react-flow__node[data-id="${id}"]`
export const card = (key) => node(sessionId(key))

/**
 * Brings storefront's board to life: its sessions running in every status
 * (sf-skeleton working with two agents working and one done, sf-tax waiting
 * on a permission, sf-cart done but running, the Run tests action working,
 * the terminal card's shell running), then opens the board.
 */
export async function storefront(app, { open = true } = {}) {
  await app.page.waitForSelector('.pv-item')
  await app.liven('storefront', ['sf-skeleton', 'sf-tax', 'sf-cart', 'sf-run-tests', 'sf-terminal'])
  await app.status('sf-skeleton', 'working')
  await app.status('sf-skeleton', 'working', agentIdOf('sf-a1'))
  await app.status('sf-skeleton', 'working', agentIdOf('sf-a2'))
  await app.status('sf-tax', 'waiting')
  await app.status('sf-run-tests', 'working')
  await app.usage('sf-cart')
  if (open) {
    await app.openProject('storefront')
    await app.page.waitForSelector(card('sf-skeleton'))
  }
}

/** Opens a project's board with nothing running. */
export async function board(app, key) {
  await app.page.waitForSelector('.pv-item')
  await app.openProject(key)
}

/** Moves the pointer somewhere neutral (the title bar's empty middle), so no hover state lingers. */
export async function rest(page) {
  await page.mouse.move(720, 26)
}

/** Seeds a board's viewport: where it opens and how far zoomed. */
export const viewport = (key, vp) => (state) => {
  state.boards[projectId(key)].viewport = vp
}

/** Combines seed mutations. */
export const seeds =
  (...fns) =>
  (state, world) => {
    for (const fn of fns) fn?.(state, world)
  }
