// Board tools: an MCP server that lets Claude sessions work with the board
// they run on. Every Claude session can write sticky notes on it, so what it
// found can go up beside its card, and put up views, small live pages, and
// change them; action runs can also list and archive the board's sessions.
//
// Claude Code reaches the server over the MCP streamable HTTP transport, at
// the session host (host.ts), which passes each request on to the app. The
// host outlives the app, as the sessions do, so a session keeps its tools
// when the app restarts. Every process gets the same config: Claude Code
// fills in the host's port and token, and the session's id, from the env
// hookEnv gives it, so the token never shows up in a process listing.
import { app } from 'electron'
import { FOLDER_COLORS, type FolderColor } from '@shared/types'
import { MCP_PATH, MCP_SESSION_HEADER } from './hostProtocol'
import { projectIdForSession } from './store'
import { VIEW_GUIDE } from './viewPage'
import type { ViewToolArgs } from './views'

/** The server's name in Claude Code: its tools are mcp__nodus__<tool>. */
export const BOARD_SERVER = 'nodus'
const PROTOCOL_VERSION = '2025-06-18'

/** What a note tool may change about a note. */
export interface NotePatch {
  text?: string
  color?: FolderColor
}

export interface BoardToolHandlers {
  /** Whether the session is an action run, which gets the session tools too. */
  isRun(sessionId: string): boolean
  /** The project's sessions, leaving out the calling run. */
  listSessions(projectId: string, self: string, includeArchived: boolean): unknown
  archiveSessions(projectId: string, self: string, sessionIds: string[], force: boolean): unknown
  /** Archives the calling session once its turn ends. */
  archiveSelf(projectId: string, self: string): unknown
  listNotes(projectId: string, self: string): unknown
  createNote(projectId: string, self: string, note: NotePatch): unknown
  updateNote(projectId: string, noteId: string, patch: NotePatch): unknown
  listViews(projectId: string, self: string): unknown
  getView(projectId: string, self: string, viewId: unknown): unknown
  createView(projectId: string, self: string, args: ViewToolArgs): Promise<unknown>
  updateView(projectId: string, self: string, args: ViewToolArgs): Promise<unknown>
}

const INSTRUCTIONS =
  'Nodus is the board of Claude Code sessions the user runs this session from. Besides the session cards, it holds sticky notes and views. When the user asks you to put something on the board, or to make notes of it, write it with create_note. Read the notes they have put up with list_notes. Only write notes when the user asks for them. When the user asks for a view, a dashboard, a table, a chart or other custom UI on the board, build it with create_view; list_views, get_view and update_view find and change the views there. When the user asks you to archive yourself, use archive_self.'

const colorProperty = (description: string) => ({ type: 'string', enum: FOLDER_COLORS, description })

/** Every session's tools. */
const COMMON_TOOLS = [
  {
    name: 'create_note',
    description:
      'Puts a sticky note on the user’s Nodus board, beside this session’s card. Use it when the user asks you to put something on the board: findings, a summary, a plan, a list to come back to. A note shows its text as written, so Markdown is not rendered: skip **bold** and # headings, though "- " bullets and blank lines read well. Keep each note to one topic someone can take in at a glance, starting with a short line that says what it is about, and write several notes for several topics. Returns the note’s id.',
    inputSchema: {
      type: 'object',
      properties: {
        text: { type: 'string', description: 'What the note says.' },
        color: colorProperty('The note’s colour. Defaults to yellow. Colours can tell kinds of notes apart.')
      },
      required: ['text']
    }
  },
  {
    name: 'update_note',
    description:
      'Changes the text or colour of a note on the board. Use it to revise a note rather than writing another beside it.',
    inputSchema: {
      type: 'object',
      properties: {
        note_id: { type: 'string', description: 'An id from create_note or list_notes.' },
        text: { type: 'string', description: 'The note’s new text, in full.' },
        color: colorProperty('The note’s new colour.')
      },
      required: ['note_id']
    }
  },
  {
    name: 'list_notes',
    description:
      'Lists the sticky notes on the board, the user’s and any Claude wrote: each one’s id, text and colour, and whether this session wrote it.',
    inputSchema: { type: 'object', properties: {} }
  },
  {
    name: 'archive_self',
    description:
      'Moves this session into the board’s archive when your current reply ends, and stops it. Its transcript is kept, so the user can pick it up again later. Use it only when the user asks you to archive yourself, and call it as the last thing you do: the session stops as soon as this turn ends.',
    inputSchema: { type: 'object', properties: {} }
  }
]

/** What a view's page, data, commands and refresh look like, for create_view and update_view. */
const VIEW_PROPERTIES = {
  title: { type: 'string', description: 'A short name for the view, shown on its card.' },
  html: {
    type: 'string',
    description: 'The page: one self-contained HTML document, with its CSS and JavaScript inline. It draws from nodus.data.'
  },
  data: {
    description:
      'JSON the page draws from, as nodus.data. Sending new data updates the page in place without reloading it.'
  },
  commands: {
    type: 'array',
    description:
      'Shell commands the page may run, replacing the ones it had. The user allows each command line once on the card; an unchanged one stays allowed.',
    items: {
      type: 'object',
      properties: {
        name: { type: 'string', description: 'What the page calls it by: nodus.run(name).' },
        run: { type: 'string', description: 'The command line, run in the project folder with the user’s shell environment.' },
        every: { type: 'number', description: 'Minutes between runs while the app is open (at least 1). Leave out to run only when the page asks.' }
      },
      required: ['name', 'run']
    }
  },
  refresh: {
    type: ['object', 'null'],
    description:
      'Asks the view’s brain to bring it up to date every so often, for data only Claude can fetch (an MCP server, say). Each refresh costs tokens. null stops it.',
    properties: {
      every: { type: 'number', description: 'Minutes between refreshes, at least 10.' },
      prompt: {
        type: 'string',
        description: 'What the brain is asked each time. It starts fresh, so say where the data comes from and to send it with update_view.'
      }
    },
    required: ['every', 'prompt']
  },
  width: { type: 'number', description: 'Card width in board units. Defaults to 480.' },
  height: { type: 'number', description: 'Card height in board units. Defaults to 360.' }
}

const VIEW_ID_PROPERTY = {
  view_id: {
    type: 'string',
    description: 'An id from create_view or list_views. A view’s brain can leave it out for its own view.'
  }
}

/** Every session's view tools. */
const VIEW_TOOLS = [
  {
    name: 'create_view',
    description: `Puts a view on the user’s Nodus board, beside this session’s card: a small live web page, such as a table, a dashboard or a chart, that stays up to date. Use it when the user asks for a view or for custom UI on the board. It returns the view’s id, the commands waiting for the user to allow them, and any JavaScript errors the page hit as it loaded. The view gets a brain of its own, a Claude session that keeps it up to date, when it first needs one.

${VIEW_GUIDE}`,
    inputSchema: {
      type: 'object',
      properties: {
        ...VIEW_PROPERTIES,
        description: {
          type: 'string',
          description: 'What the view is for and where its data comes from, for its brain to go on later.'
        }
      },
      required: ['title', 'html']
    }
  },
  {
    name: 'update_view',
    description:
      'Changes a view on the board: its page, its data, its title, its commands or its refresh schedule. Leave out what stays the same. To show fresh data, send only data. Returns the commands waiting for the user to allow them and, when the page changed, any JavaScript errors it hit as it loaded.',
    inputSchema: { type: 'object', properties: { ...VIEW_ID_PROPERTY, ...VIEW_PROPERTIES } }
  },
  {
    name: 'get_view',
    description:
      'Reads a view as it is now: its page, data and saved state, its commands with whether the user allowed them and their last output, its refresh schedule, and the JavaScript errors it hit lately.',
    inputSchema: { type: 'object', properties: VIEW_ID_PROPERTY }
  },
  {
    name: 'list_views',
    description: 'Lists the views on the board: each one’s id, title and description, and whether it’s yours.',
    inputSchema: { type: 'object', properties: {} }
  }
]

/** An action run's tools, on top of the common ones. */
const SESSION_TOOLS = [
  {
    name: 'list_sessions',
    description:
      'Lists the sessions on this project’s Nodus board. Each has its id, title, kind (claude or terminal), status (working, waiting for the user, or done), whether its process is running, whether it is archived, the section it is in, Claude’s last reply, and when it was last active. The session running this action is not listed.',
    inputSchema: {
      type: 'object',
      properties: {
        include_archived: { type: 'boolean', description: 'Also list archived sessions. Defaults to false.' }
      }
    }
  },
  {
    name: 'archive_sessions',
    description:
      'Moves sessions into the board’s archive and stops their processes. Their transcripts are kept, so they can be picked up again later. A session nothing was said in is removed from the board instead, as there is nothing to keep. Sessions that are working or waiting for the user are skipped unless force is true. Terminals cannot be archived.',
    inputSchema: {
      type: 'object',
      properties: {
        session_ids: { type: 'array', items: { type: 'string' }, description: 'Ids from list_sessions.' },
        force: { type: 'boolean', description: 'Archive sessions even if they are working or waiting.' }
      },
      required: ['session_ids']
    }
  }
]

const SESSION_TOOL_NAMES = new Set(SESSION_TOOLS.map((t) => t.name))

/** Allow rules that let a session use every board tool it has without asking. */
export const BOARD_TOOL_RULES = [...COMMON_TOOLS, ...VIEW_TOOLS, ...SESSION_TOOLS].map(
  (t) => `mcp__${BOARD_SERVER}__${t.name}`
)

/** The `--mcp-config` for every claude process. Claude Code expands the ${…} from its env (hookEnv). */
export const BOARD_MCP_CONFIG = JSON.stringify({
  mcpServers: {
    [BOARD_SERVER]: {
      type: 'http',
      url: `http://127.0.0.1:\${NODUS_PORT}${MCP_PATH}`,
      headers: {
        Authorization: 'Bearer ${NODUS_TOKEN}',
        [MCP_SESSION_HEADER]: '${NODUS_SESSION}'
      }
    }
  }
})

let handlers: BoardToolHandlers | null = null

export function configureBoardTools(h: BoardToolHandlers) {
  handlers = h
}

interface RpcRequest {
  id: string | number | null
  method: string
  params?: Record<string, unknown>
}

/**
 * Answers a JSON-RPC request from a session's process, relayed by the session
 * host. A tool call answers once it's done, which for a view's page means once
 * the board has loaded it.
 */
export function answerMcp(sessionId: string, message: unknown): unknown {
  const msg = message as RpcRequest
  const ok = (result: unknown) => ({ jsonrpc: '2.0', id: msg.id, result })
  switch (msg.method) {
    case 'initialize': {
      const asked = msg.params?.protocolVersion
      return ok({
        protocolVersion: typeof asked === 'string' ? asked : PROTOCOL_VERSION,
        capabilities: { tools: {} },
        serverInfo: { name: BOARD_SERVER, version: app.getVersion() },
        instructions: INSTRUCTIONS
      })
    }
    case 'ping':
      return ok({})
    case 'tools/list':
      return ok({
        tools: handlers?.isRun(sessionId)
          ? [...COMMON_TOOLS, ...VIEW_TOOLS, ...SESSION_TOOLS]
          : [...COMMON_TOOLS, ...VIEW_TOOLS]
      })
    case 'tools/call':
      return callTool(msg.params ?? {}, sessionId).then(ok)
    default:
      return { jsonrpc: '2.0', id: msg.id, error: { code: -32601, message: `Unknown method ${msg.method}` } }
  }
}

const isColor = (value: unknown): value is FolderColor => FOLDER_COLORS.includes(value as FolderColor)

async function callTool(params: Record<string, unknown>, sessionId: string) {
  const args = (params.arguments && typeof params.arguments === 'object' ? params.arguments : {}) as Record<
    string,
    unknown
  >
  const text = (value: unknown) => ({ content: [{ type: 'text', text: JSON.stringify(value, null, 2) }] })
  const patch = (): NotePatch => ({
    text: typeof args.text === 'string' ? args.text : undefined,
    color: isColor(args.color) ? args.color : undefined
  })
  try {
    if (!handlers) throw new Error('The board is not available')
    const projectId = projectIdForSession(sessionId)
    if (!projectId) throw new Error('This session is no longer on a Nodus board')
    // A run the user took over is an ordinary session now.
    if (SESSION_TOOL_NAMES.has(String(params.name)) && !handlers.isRun(sessionId)) {
      throw new Error(`${String(params.name)} is only for action runs`)
    }
    switch (params.name) {
      case 'create_note':
        return text(handlers.createNote(projectId, sessionId, patch()))
      case 'update_note':
        if (typeof args.note_id !== 'string') throw new Error('note_id is required')
        return text(handlers.updateNote(projectId, args.note_id, patch()))
      case 'list_notes':
        return text(handlers.listNotes(projectId, sessionId))
      case 'list_sessions':
        return text(handlers.listSessions(projectId, sessionId, args.include_archived === true))
      case 'archive_sessions': {
        const ids = Array.isArray(args.session_ids) ? args.session_ids.filter((id) => typeof id === 'string') : []
        return text(handlers.archiveSessions(projectId, sessionId, ids, args.force === true))
      }
      case 'archive_self':
        return text(handlers.archiveSelf(projectId, sessionId))
      case 'list_views':
        return text(handlers.listViews(projectId, sessionId))
      case 'get_view':
        return text(handlers.getView(projectId, sessionId, args.view_id))
      case 'create_view':
        return text(await handlers.createView(projectId, sessionId, args))
      case 'update_view':
        return text(await handlers.updateView(projectId, sessionId, args))
      default:
        throw new Error(`Unknown tool ${String(params.name)}`)
    }
  } catch (err) {
    return { content: [{ type: 'text', text: err instanceof Error ? err.message : String(err) }], isError: true }
  }
}
