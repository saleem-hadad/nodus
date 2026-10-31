import { beforeEach, describe, expect, it, vi } from 'vitest'
import { BOARD_MCP_CONFIG, BOARD_TOOL_RULES, answerMcp, configureBoardTools, type BoardToolHandlers } from './boardTools'

vi.mock('electron', () => ({ app: { getVersion: () => '9.9.9' } }))
// Every session is on project p1, except "gone", which has left the board.
vi.mock('./store', () => ({ projectIdForSession: (id: string) => (id === 'gone' ? null : 'p1') }))

const RUN = 'run-session'

let handlers: { [K in keyof BoardToolHandlers]: ReturnType<typeof vi.fn> }

beforeEach(() => {
  handlers = {
    isRun: vi.fn((id: string) => id === RUN),
    listSessions: vi.fn(() => [{ id: 's2' }]),
    archiveSessions: vi.fn(() => ({ archived: [] })),
    archiveSelf: vi.fn(() => ({ archiving: 'later' })),
    listNotes: vi.fn(() => []),
    createNote: vi.fn(() => ({ noteId: 'n1' })),
    updateNote: vi.fn(() => ({ noteId: 'n1' }))
  }
  configureBoardTools(handlers as unknown as BoardToolHandlers)
})

const call = (sessionId: string, name: string, args?: unknown) =>
  answerMcp(sessionId, { jsonrpc: '2.0', id: 7, method: 'tools/call', params: { name, arguments: args } }) as {
    id: number
    result: { content: { type: string; text: string }[]; isError?: boolean }
  }

const toolNames = (sessionId: string) =>
  (answerMcp(sessionId, { id: 1, method: 'tools/list' }) as { result: { tools: { name: string }[] } }).result.tools.map(
    (t) => t.name
  )

describe('protocol', () => {
  it('answers initialize in the version asked for', () => {
    const reply = answerMcp('s1', { id: 1, method: 'initialize', params: { protocolVersion: '2025-03-26' } })
    expect(reply).toMatchObject({
      jsonrpc: '2.0',
      id: 1,
      result: {
        protocolVersion: '2025-03-26',
        capabilities: { tools: {} },
        serverInfo: { name: 'nodus', version: '9.9.9' },
        instructions: expect.stringContaining('create_note')
      }
    })
  })

  it('answers ping, and unknown methods with an error', () => {
    expect(answerMcp('s1', { id: 2, method: 'ping' })).toEqual({ jsonrpc: '2.0', id: 2, result: {} })
    expect(answerMcp('s1', { id: 3, method: 'resources/list' })).toEqual({
      jsonrpc: '2.0',
      id: 3,
      error: { code: -32601, message: 'Unknown method resources/list' }
    })
  })

  it('gives every session the note tools, and action runs the session tools too', () => {
    expect(toolNames('s1')).toEqual(['create_note', 'update_note', 'list_notes', 'archive_self'])
    expect(toolNames(RUN)).toEqual([
      'create_note',
      'update_note',
      'list_notes',
      'archive_self',
      'list_sessions',
      'archive_sessions'
    ])
  })

  it('allows every tool by name and keeps the token out of the config', () => {
    expect(BOARD_TOOL_RULES).toEqual([
      'mcp__nodus__create_note',
      'mcp__nodus__update_note',
      'mcp__nodus__list_notes',
      'mcp__nodus__archive_self',
      'mcp__nodus__list_sessions',
      'mcp__nodus__archive_sessions'
    ])
    const { url, headers } = JSON.parse(BOARD_MCP_CONFIG).mcpServers.nodus
    expect(url).toBe('http://127.0.0.1:${NODUS_PORT}/mcp')
    expect(headers).toEqual({ Authorization: 'Bearer ${NODUS_TOKEN}', 'X-Nodus-Session': '${NODUS_SESSION}' })
  })
})

describe('tools', () => {
  it('creates a note, dropping a colour that is not one of the board’s', () => {
    const reply = call('s1', 'create_note', { text: 'Findings', color: 'chartreuse' })
    expect(handlers.createNote).toHaveBeenCalledWith('p1', 's1', { text: 'Findings', color: undefined })
    expect(reply.id).toBe(7)
    expect(JSON.parse(reply.result.content[0].text)).toEqual({ noteId: 'n1' })
  })

  it('updates a note by id', () => {
    call('s1', 'update_note', { note_id: 'n1', color: 'green' })
    expect(handlers.updateNote).toHaveBeenCalledWith('p1', 'n1', { text: undefined, color: 'green' })
    expect(call('s1', 'update_note', { text: 'no id' }).result).toMatchObject({
      isError: true,
      content: [{ text: 'note_id is required' }]
    })
  })

  it('lists notes and archives the calling session', () => {
    call('s1', 'list_notes')
    expect(handlers.listNotes).toHaveBeenCalledWith('p1', 's1')
    call('s1', 'archive_self')
    expect(handlers.archiveSelf).toHaveBeenCalledWith('p1', 's1')
  })

  it('keeps the session tools to action runs', () => {
    expect(call('s1', 'list_sessions').result).toMatchObject({
      isError: true,
      content: [{ text: 'list_sessions is only for action runs' }]
    })
    expect(call('s1', 'archive_sessions', { session_ids: ['s2'] }).result.isError).toBe(true)
    expect(handlers.listSessions).not.toHaveBeenCalled()
    expect(handlers.archiveSessions).not.toHaveBeenCalled()
  })

  it('lets an action run list and archive sessions', () => {
    call(RUN, 'list_sessions', { include_archived: 'yes' })
    expect(handlers.listSessions).toHaveBeenCalledWith('p1', RUN, false)
    call(RUN, 'archive_sessions', { session_ids: ['s2', 3, null, 's3'], force: true })
    expect(handlers.archiveSessions).toHaveBeenCalledWith('p1', RUN, ['s2', 's3'], true)
  })

  it('turns failures into tool errors Claude can read', () => {
    handlers.createNote.mockImplementation(() => {
      throw new Error('A note needs some text')
    })
    expect(call('s1', 'create_note', { text: '' }).result).toEqual({
      content: [{ type: 'text', text: 'A note needs some text' }],
      isError: true
    })
    expect(call('gone', 'list_notes').result.content[0].text).toBe('This session is no longer on a Nodus board')
    expect(call('s1', 'delete_everything').result.content[0].text).toBe('Unknown tool delete_everything')
  })
})
