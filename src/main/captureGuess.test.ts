import { describe, expect, it, vi } from 'vitest'

vi.mock('./env', () => ({ requireClaude: () => 'claude', spawnEnv: () => ({}) }))

import type { CaptureTargets } from '@shared/types'
import { buildGuessPrompt, readGuess } from './captureGuess'

const now = 1_800_000_000_000

const targets: CaptureTargets = {
  projects: [
    { id: 'p-store', name: 'storefront', color: 'blue' },
    { id: 'p-api', name: 'api', color: 'purple' }
  ],
  sessions: [
    {
      projectId: 'p-store',
      sessionId: 's-round',
      title: 'Fix checkout total rounding',
      status: 'done',
      live: true,
      updatedAt: now - 25 * 60_000
    },
    {
      projectId: 'p-api',
      sessionId: 's-uuid',
      title: 'Migrate orders to UUIDs',
      status: 'waiting',
      live: true,
      updatedAt: now - 2 * 60_000
    },
    {
      projectId: 'p-api',
      sessionId: 's-stripe',
      title: 'Bump Stripe SDK',
      status: 'working',
      live: true,
      updatedAt: now
    }
  ],
  recents: [],
  defaultProjectId: 'p-store'
}

describe('buildGuessPrompt', () => {
  it('lists projects with their sessions, the screen and the text', () => {
    const prompt = buildGuessPrompt(
      {
        text: 'also cover the coupon path',
        app: 'Safari',
        attachments: [
          {
            kind: 'tab',
            app: 'Safari',
            title: 'Refund fails #482',
            url: 'https://github.com/acme/storefront/issues/482'
          },
          { kind: 'file', path: '/x/checkout.log', name: 'checkout.log', size: 3 }
        ],
        targets
      },
      now
    )
    expect(prompt).toContain('Projects (default: storefront, p-store):')
    expect(prompt).toContain('- project p-store: storefront')
    expect(prompt).toContain('  - session s-round: “Fix checkout total rounding”, idle, active 25 min ago')
    expect(prompt).toContain('  - session s-uuid: “Migrate orders to UUIDs”, waiting for the user, active 2 min ago')
    expect(prompt).toContain('The app in front: Safari')
    expect(prompt).toContain('Its browser tab: Refund fails #482 (https://github.com/acme/storefront/issues/482)')
    expect(prompt).toContain('A file is attached: checkout.log')
    expect(prompt).toMatch(/The capture:\n"""\nalso cover the coupon path\n"""$/)
  })
})

describe('readGuess', () => {
  it('takes a valid answer', () => {
    expect(
      readGuess({ kind: 'into', project: 'p-store', session: 's-round', reason: 'It follows on.' }, targets)
    ).toEqual({
      kind: 'into',
      projectId: 'p-store',
      sessionId: 's-round',
      reason: 'It follows on.'
    })
  })

  it('falls back to the default project for one it made up', () => {
    expect(readGuess({ kind: 'note', project: 'nope', session: '', reason: 'An idea.' }, targets)).toMatchObject({
      kind: 'note',
      projectId: 'p-store'
    })
  })

  it('starts a session instead of sending into one that is waiting or missing', () => {
    expect(readGuess({ kind: 'into', project: 'p-api', session: 's-uuid', reason: '' }, targets)).toMatchObject({
      kind: 'session',
      projectId: 'p-api',
      sessionId: undefined
    })
    expect(readGuess({ kind: 'into', project: 'p-api', session: 'gone', reason: '' }, targets)?.kind).toBe('session')
  })

  it("takes the session's own project", () => {
    expect(readGuess({ kind: 'into', project: 'p-store', session: 's-stripe', reason: '' }, targets)?.projectId).toBe(
      'p-api'
    )
  })

  it('turns down nonsense', () => {
    expect(readGuess(null, targets)).toBeNull()
    expect(readGuess({ kind: 'party', project: 'p-api', session: '', reason: 7 }, targets)).toEqual({
      kind: 'session',
      projectId: 'p-api',
      sessionId: undefined,
      reason: ''
    })
  })
})
