import { describe, expect, it } from 'vitest'
import { matchCommands, takeCommand, withContext } from './compose'

describe('matchCommands', () => {
  it('offers every command for a lone slash', () => {
    expect(matchCommands('/').map((c) => c.command)).toEqual(['/view', '/note'])
  })

  it('narrows as the word is typed, in any case', () => {
    expect(matchCommands('/V').map((c) => c.command)).toEqual(['/view'])
    expect(matchCommands('/note').map((c) => c.command)).toEqual(['/note'])
  })

  it('offers nothing for other words, or once there is a space', () => {
    expect(matchCommands('/x')).toEqual([])
    expect(matchCommands('/view ')).toEqual([])
    expect(matchCommands('fix the /view')).toEqual([])
    expect(matchCommands('')).toEqual([])
  })
})

describe('takeCommand', () => {
  it('takes a command followed by a space, with what comes after', () => {
    expect(takeCommand('/view ')).toEqual({ target: 'view', rest: '' })
    expect(takeCommand('/NOTE  ask finance\nby Friday')).toEqual({ target: 'note', rest: 'ask finance\nby Friday' })
  })

  it('leaves a command still being typed, and anything else', () => {
    expect(takeCommand('/view')).toBeNull()
    expect(takeCommand('/views of x')).toBeNull()
    expect(takeCommand('add a /view')).toBeNull()
  })
})

describe('withContext', () => {
  it('leaves a prompt alone when nothing is selected', () => {
    expect(withContext([], 'Fix it')).toBe('Fix it')
  })

  it('writes the selection out before the prompt', () => {
    const prompt = withContext(
      [
        {
          kind: 'session',
          title: 'Fix cart rounding',
          branch: 'fix/cart-rounding',
          lastReply: 'All 6 tests pass.\n\nDone.',
          transcript: '/t/a.jsonl'
        },
        { kind: 'session', title: 'New session' },
        { kind: 'note', text: ' EUR rounding: ask finance ' },
        { kind: 'view', title: 'Tests', description: 'Test results by file' }
      ],
      'Why does cart.test.js fail?'
    )
    expect(prompt).toBe(
      [
        'For context, I selected these on my board:',
        '',
        '## Session “Fix cart rounding”',
        'Branch: fix/cart-rounding',
        'Its last reply:',
        '> All 6 tests pass.',
        '>',
        '> Done.',
        'Its whole conversation, if you need more: /t/a.jsonl',
        '',
        '## Session “New session”',
        'It has no replies yet.',
        '',
        '## Note',
        'EUR rounding: ask finance',
        '',
        '## View “Tests”',
        'Test results by file',
        '',
        '---',
        '',
        'Why does cart.test.js fail?'
      ].join('\n')
    )
  })
})
