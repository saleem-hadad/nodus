import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { SessionStatus } from '@shared/types'
import { noticeFor, waitingCount } from './notify'
import { getBoard } from './store'

vi.mock('electron', () => ({ app: {}, Notification: class {} }))
vi.mock('./store', async (importOriginal) => ({ ...(await importOriginal<typeof import('./store')>()), save: vi.fn() }))

const statuses = vi.hoisted(() => new Map<string, 'working' | 'waiting' | 'done'>())
vi.mock('./pty', () => ({ isLive: (id: string) => statuses.has(id) }))
vi.mock('./hooks', () => ({ getStatus: (id: string) => statuses.get(id) ?? 'done' }))

describe('noticeFor', () => {
  it('tells when a session starts needing the user', () => {
    expect(noticeFor('working', 'waiting', true)).toBe('waiting')
    expect(noticeFor('done', 'waiting', true)).toBe('waiting')
    expect(noticeFor(undefined, 'waiting', true)).toBe('waiting')
  })

  it('tells once while it keeps waiting', () => {
    expect(noticeFor('waiting', 'waiting', true)).toBeNull()
  })

  it('tells when a turn ends', () => {
    expect(noticeFor('working', 'done', true)).toBe('finished')
  })

  it('stays quiet when a process goes, or nothing was running', () => {
    expect(noticeFor('working', 'done', false)).toBeNull()
    expect(noticeFor('waiting', 'done', true)).toBeNull()
    expect(noticeFor(undefined, 'done', true)).toBeNull()
    expect(noticeFor(undefined, 'working', true)).toBeNull()
    expect(noticeFor('waiting', 'working', true)).toBeNull()
  })
})

describe('waitingCount', () => {
  beforeEach(() => statuses.clear())

  const card = (sessionId: string, status: SessionStatus | null, extra = {}) => {
    getBoard('a').nodes[sessionId] = { sessionId, x: 0, y: 0, archived: false, createdAt: 0, ...extra }
    if (status) statuses.set(sessionId, status)
  }

  it('counts the waiting sessions on every board, action runs too', () => {
    card('w1', 'waiting')
    card('w2', 'working')
    card('r1', 'waiting', { runOf: 'act' })
    getBoard('b').nodes.w3 = { sessionId: 'w3', x: 0, y: 0, archived: false, createdAt: 0 }
    statuses.set('w3', 'waiting')
    expect(waitingCount()).toBe(3)
  })

  it('leaves out views’ brains, archived cards and sessions that aren’t running', () => {
    card('v1', 'waiting', { viewOf: 'view' })
    card('x1', 'waiting', { archived: true })
    card('d1', null)
    expect(waitingCount()).toBe(0)
  })
})
