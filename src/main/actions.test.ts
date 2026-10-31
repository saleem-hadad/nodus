import { describe, expect, it } from 'vitest'
import type { ProjectAction } from '@shared/types'
import { normalizeAction, runArgs } from './actions'

describe('normalizeAction', () => {
  it('needs a prompt', () => {
    expect(() => normalizeAction({ name: 'Test' })).toThrow('An action needs a prompt')
    expect(() => normalizeAction({ prompt: '   ' })).toThrow('An action needs a prompt')
  })

  it('fills in a new action', () => {
    const action = normalizeAction({ prompt: '  Run the tests.  ' })
    expect(action).toEqual({
      id: expect.stringMatching(/^[0-9a-f-]{36}$/),
      name: 'Action',
      icon: 'zap',
      description: '',
      prompt: 'Run the tests.',
      global: undefined,
      createdAt: expect.any(Number)
    })
  })

  it('trims and clips what the renderer sends', () => {
    const action = normalizeAction({
      name: `  ${'N'.repeat(60)}  `,
      prompt: 'p'.repeat(25_000),
      description: 'd'.repeat(5000),
      icon: 'not-an-icon' as ProjectAction['icon'],
      global: 'yes' as unknown as boolean
    })
    expect(action.name).toHaveLength(40)
    expect(action.prompt).toHaveLength(20_000)
    expect(action.description).toHaveLength(4000)
    expect(action.icon).toBe('zap')
    expect(action.global).toBeUndefined()
  })

  it('keeps an existing action’s id and creation time', () => {
    const existing = normalizeAction({ prompt: 'Old', icon: 'bug', global: true })
    const edited = normalizeAction({ id: 'other', prompt: 'New', icon: 'rocket', global: true }, existing)
    expect(edited).toMatchObject({ id: existing.id, createdAt: existing.createdAt, prompt: 'New', icon: 'rocket', global: true })
  })
})

describe('runArgs', () => {
  it('tells the run which action it is', () => {
    const args = runArgs(normalizeAction({ name: 'Tidy Up', prompt: 'Tidy.' }))
    expect(args[0]).toBe('--append-system-prompt')
    expect(args[1]).toContain('"Tidy Up"')
  })
})
