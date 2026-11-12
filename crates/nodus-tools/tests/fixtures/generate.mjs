// Writes tools.json beside this file from the TypeScript itself, so the Rust
// tests check the board tools' MCP answers byte for byte against 0.9.4's,
// on macOS and on Windows, with the tools added since spliced in (ADDED_*
// below). Every id here is made up.
//
// The committed tools.json was generated from 0.9.4's sources. The Electron
// app has since left this repo, so to regenerate it, point NODUS_V094 at a
// checkout of tag v0.9.4 and run, from this repo's root, with Node 22:
//
//   git worktree add ../nodus-0.9.4 v0.9.4
//   NODUS_V094=../nodus-0.9.4 node --experimental-strip-types crates/nodus-tools/tests/fixtures/generate.mjs
//
// It copies src/main/boardTools.ts and what it imports to a temporary folder,
// swapping only Electron's app and the store for stubs.
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
if (!process.env.NODUS_V094) throw new Error('Set NODUS_V094 to a checkout of tag v0.9.4 (see the top of this file)')
const root = path.resolve(process.env.NODUS_V094)

function copy(work, from, name, replacements = []) {
  let source = fs.readFileSync(path.join(root, from), 'utf8')
  for (const [find, to] of replacements) {
    if (!source.includes(find)) throw new Error(`${from} no longer has: ${find}`)
    source = source.replace(find, to)
  }
  fs.writeFileSync(path.join(work, `${name}.mts`), source)
}

/** boardTools.ts as it loads on `platform`. */
async function load(platform) {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), `nodus-tools-${platform}-`))
  copy(work, 'src/shared/types.ts', 'types')
  copy(work, 'src/main/hostProtocol.ts', 'hostProtocol')
  copy(work, 'src/main/viewPage.ts', 'viewPage', [
    [`process.platform === 'win32'`, `${JSON.stringify(platform)} === 'win32'`]
  ])
  copy(work, 'src/main/boardTools.ts', 'boardTools', [
    [`import { app } from 'electron'`, `const app = { getVersion: () => '9.9.9' }`],
    [`from '@shared/types'`, `from './types.mts'`],
    [`from './hostProtocol'`, `from './hostProtocol.mts'`],
    [
      `import { projectIdForSession } from './store'`,
      `const projectIdForSession = (id: string) => (id === 'gone' ? null : 'p1')`
    ],
    [`from './viewPage'`, `from './viewPage.mts'`]
  ])
  const tools = await import(pathToFileURL(path.join(work, 'boardTools.mts')).href)
  const page = await import(pathToFileURL(path.join(work, 'viewPage.mts')).href)
  return { tools, page }
}

/** A value with what JSON.stringify writes in its own way: big and fractional numbers, -0, escapes, empties. */
const AWKWARD = {
  at: 1_700_000_000_000,
  half: 0.5,
  tiny: 1e-7,
  huge: 1e21,
  negativeZero: -0,
  text: 'quote " backslash \\ tab \t newline \n control \u0001 del \u007f é   😀',
  empty: {},
  none: [],
  nested: [{ a: [1, [2, {}]] }],
  nil: null
}

const RUN = 'run-session'
const handlers = {
  isRun: (id) => id === RUN,
  listProjects: () => AWKWARD,
  createProject: () => ({ projectId: 'p3' }),
  findProject: () => 'p1',
  listSessions: () => [],
  archiveSessions: () => ({ archived: [], removed: [], skipped: [] }),
  archiveSelf: () => ({ archiving: 'This session moves to the archive when this turn ends.' }),
  listNotes: () => [],
  createNote: () => ({ noteId: 'n1' }),
  updateNote: () => ({ noteId: 'n1' }),
  listViews: () => [],
  getView: () => ({ id: 'v1' }),
  createView: async () => ({ view_id: 'v1' }),
  updateView: async () => ({ view_id: 'v1', version: 2 })
}

// The board tools have grown since 0.9.4: the drawing tools (TOOL-101 to
// TOOL-110 in spec 04) come after archive_self, in the tool lists and in the
// allow rules, and the instructions end with a few words on them. They are
// written here as the Rust app gives them, and spliced into 0.9.4's answers.
const ADDED_AFTER = 'archive_self'
const ADDED_INSTRUCTIONS = " The board also holds shapes (boxes, ellipses, diamonds, words and emoji stickers) and links, arrows between cards, notes, views and shapes. The user links cards to show how their work relates: list_board shows what is on the board and where, and what this session’s card is linked to. When the user asks for something drawn, laid out, rearranged or linked on the board, use draw_on_board, update_drawing and erase_from_board."
const ADDED_TOOLS = [
  {
    "name": "list_board",
    "description": "Shows what is on this session’s Nodus board and where: the session cards (one of them this session’s), sticky notes, views and shapes, each with its id and its box (x and y are its top left corner in board units, y growing downward), and the links between them. linked_to_this_session says in words what this session’s card is linked to and from. The user links cards to show how their work relates, so look there to see which work bears on yours. Use it before drawing on the board, rearranging it or linking things on it.",
    "inputSchema": {
      "type": "object",
      "properties": {}
    }
  },
  {
    "name": "draw_on_board",
    "description": "Draws on the user’s Nodus board, as on a whiteboard: boxes, ellipses, diamonds, words and emoji stickers, and links, arrows between them and the cards, notes and views already there. Use it only when the user asks for something drawn, laid out or linked on the board, such as a diagram, a flow, or how pieces of work relate. Give shapes’ x and y from the drawing’s own top left: the whole drawing goes in the nearest free spot beside this session’s card, unless at puts it somewhere. Shapes without x and y go in a row, left to right. Returns the new shapes’ ids by key (or by their place in shapes, from 0), the new links’ ids, and the area the drawing took.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "shapes": {
          "type": "array",
          "description": "The shapes to draw.",
          "items": {
            "type": "object",
            "properties": {
              "key": {
                "type": "string",
                "description": "A name for the shape in this call, for links to point at it by. Its id comes back under it."
              },
              "kind": {
                "type": "string",
                "enum": [
                  "rect",
                  "ellipse",
                  "diamond",
                  "text",
                  "sticker"
                ],
                "description": "rect, ellipse and diamond are outlines with words inside; text is words on their own; sticker is an emoji."
              },
              "text": {
                "type": "string",
                "description": "The words in it, or a sticker’s emoji."
              },
              "color": {
                "type": "string",
                "enum": [
                  "graphite",
                  "red",
                  "orange",
                  "green",
                  "blue",
                  "purple"
                ],
                "description": "Its colour. Defaults to graphite."
              },
              "x": {
                "type": "number",
                "description": "Its left edge, from the drawing’s left. Give x and y, or neither to put it in the row."
              },
              "y": {
                "type": "number",
                "description": "Its top edge, from the drawing’s top."
              },
              "w": {
                "type": "number",
                "description": "Its width. Defaults to 180 for a rect or an ellipse, 160 for a diamond, 220 for text and 64 for a sticker."
              },
              "h": {
                "type": "number",
                "description": "Its height. Defaults to 90 for a rect or an ellipse, 110 for a diamond, 44 for text and 64 for a sticker."
              }
            },
            "required": [
              "kind"
            ]
          }
        },
        "links": {
          "type": "array",
          "description": "Arrows to draw.",
          "items": {
            "type": "object",
            "properties": {
              "from": {
                "type": "string",
                "description": "What the arrow starts at: a key from shapes, the id of a card (its session id), note, view or shape from list_board, or \"self\" for this session’s card."
              },
              "to": {
                "type": "string",
                "description": "What the arrow points at, given the same way."
              },
              "label": {
                "type": "string",
                "description": "A few words on the arrow."
              },
              "color": {
                "type": "string",
                "enum": [
                  "graphite",
                  "red",
                  "orange",
                  "green",
                  "blue",
                  "purple"
                ],
                "description": "Its colour. Defaults to graphite."
              }
            },
            "required": [
              "from",
              "to"
            ]
          }
        },
        "at": {
          "type": "object",
          "description": "Where the drawing’s top left goes, in board coordinates as list_board gives them. Leave it out to put the drawing beside this session’s card.",
          "properties": {
            "x": {
              "type": "number"
            },
            "y": {
              "type": "number"
            }
          },
          "required": [
            "x",
            "y"
          ]
        }
      }
    }
  },
  {
    "name": "update_drawing",
    "description": "Changes what is on the board: a shape’s place, size, words or colour, a link’s label, colour or ends, and where cards, notes and views are, which it only moves. Positions are board coordinates, as list_board gives them. Use it only when the user asks for the board to be changed, rearranged or regrouped, or for something drawn on it to be fixed. Returns the ids it changed.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "items": {
          "type": "array",
          "description": "Shapes to change, and cards, notes and views to move.",
          "items": {
            "type": "object",
            "properties": {
              "id": {
                "type": "string",
                "description": "The id of a shape, card (its session id), note or view, from list_board."
              },
              "x": {
                "type": "number",
                "description": "Its new left edge."
              },
              "y": {
                "type": "number",
                "description": "Its new top edge."
              },
              "w": {
                "type": "number",
                "description": "A shape’s new width."
              },
              "h": {
                "type": "number",
                "description": "A shape’s new height."
              },
              "text": {
                "type": "string",
                "description": "A shape’s new words, or a sticker’s new emoji."
              },
              "color": {
                "type": "string",
                "enum": [
                  "graphite",
                  "red",
                  "orange",
                  "green",
                  "blue",
                  "purple"
                ],
                "description": "A shape’s new colour."
              }
            },
            "required": [
              "id"
            ]
          }
        },
        "links": {
          "type": "array",
          "description": "Links to change.",
          "items": {
            "type": "object",
            "properties": {
              "id": {
                "type": "string",
                "description": "A link’s id, from list_board or draw_on_board."
              },
              "label": {
                "type": "string",
                "description": "Its new words. An empty string takes them off."
              },
              "color": {
                "type": "string",
                "enum": [
                  "graphite",
                  "red",
                  "orange",
                  "green",
                  "blue",
                  "purple"
                ],
                "description": "Its new colour."
              },
              "from": {
                "type": "string",
                "description": "What it starts at now: the id of a card (its session id), note, view or shape from list_board, or \"self\" for this session’s card."
              },
              "to": {
                "type": "string",
                "description": "What it points at now, given the same way."
              }
            },
            "required": [
              "id"
            ]
          }
        }
      }
    }
  },
  {
    "name": "erase_from_board",
    "description": "Takes shapes and links off the board. A shape’s links go with it. Cards, notes and views can’t be erased. Use it only when the user asks for something drawn on the board to go. Returns what was erased, and what was skipped and why.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "ids": {
          "type": "array",
          "items": {
            "type": "string"
          },
          "description": "Ids of shapes and links, from list_board or draw_on_board."
        }
      },
      "required": [
        "ids"
      ]
    }
  }
]

/** An answer of 0.9.4's with the tools added since. */
function withAdded(answer) {
  const reply = JSON.parse(answer)
  const result = reply.result
  if (typeof result.instructions === 'string') result.instructions += ADDED_INSTRUCTIONS
  if (Array.isArray(result.tools)) {
    const at = result.tools.findIndex((t) => t.name === ADDED_AFTER) + 1
    result.tools.splice(at, 0, ...ADDED_TOOLS)
  }
  return JSON.stringify(reply)
}

function rulesWithAdded(rules) {
  const at = rules.indexOf(`mcp__nodus__${ADDED_AFTER}`) + 1
  return [...rules.slice(0, at), ...ADDED_TOOLS.map((t) => `mcp__nodus__${t.name}`), ...rules.slice(at)]
}

const out = {}
for (const platform of ['darwin', 'win32']) {
  const { tools, page } = await load(platform)
  tools.configureBoardTools(handlers)
  const answer = async (sessionId, message) => JSON.stringify(await tools.answerMcp(sessionId, message))
  out[platform] = {
    viewGuide: page.VIEW_GUIDE,
    initialize: withAdded(await answer('s1', { jsonrpc: '2.0', id: 1, method: 'initialize', params: {} })),
    initializeAsked: withAdded(
      await answer('s1', { id: 'one', method: 'initialize', params: { protocolVersion: '2025-03-26' } })
    ),
    toolsListSession: withAdded(await answer('s1', { id: 2, method: 'tools/list' })),
    toolsListRun: withAdded(await answer(RUN, { id: 3, method: 'tools/list' })),
    noId: await answer('s1', { method: 'ping' }),
    nullId: await answer('s1', { id: null, method: 'ping' }),
    unknown: await answer('s1', { id: 4, method: 'resources/list' }),
    pretty: await answer('s1', { id: 5, method: 'tools/call', params: { name: 'list_projects' } }),
    unknownTool: await answer('s1', { id: 6, method: 'tools/call', params: {} }),
    gone: await answer('gone', { id: 7, method: 'tools/call', params: { name: 'list_notes', arguments: [] } })
  }
  out.mcpConfig = tools.BOARD_MCP_CONFIG
  out.toolRules = rulesWithAdded(tools.BOARD_TOOL_RULES)
}

fs.writeFileSync(path.join(here, 'tools.json'), JSON.stringify(out, null, 2) + '\n')
console.log('wrote tools.json')
