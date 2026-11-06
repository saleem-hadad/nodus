// Messages between the app (pty.ts) and the session host (host.ts): one JSON
// object per line over a Unix socket, or a named pipe on Windows.
import fs from 'fs'
import os from 'os'
import path from 'path'
import crypto from 'crypto'
import { StringDecoder } from 'string_decoder'
import type { SessionStatus } from '@shared/types'

/**
 * Bumped whenever a message changes shape. The app only talks to a host that
 * speaks its version. `hello`, `welcome.protocol` and `shutdown` never change,
 * so any app can tell an older host apart and stop it.
 */
export const PROTOCOL = 1

/**
 * The event name Claude's status line reports arrive under, beside the hooks.
 * They come after every reply, so only the latest is kept, apart from history.
 */
export const STATUS_LINE_EVENT = 'StatusLine'

/** Hooks and the keystrokes status depends on, in the order they happened, with when. */
export type HistoryEntry = ({ hook: string; payload: Record<string, unknown> } | { input: string }) & { at: number }

export interface HostSession {
  sessionId: string
  projectId: string
  /** Status when the process started. */
  initial: SessionStatus
  /** Recent output, for replaying into a terminal. */
  buffer: string
  /** Everything that has moved its status since the process started, oldest first. */
  history: HistoryEntry[]
  /**
   * Claude's latest status line report. Optional, as an older host doesn't
   * keep one; that needs no new protocol, and running sessions carry on.
   */
  statusLine?: { payload: Record<string, unknown>; at: number }
}

export interface SpawnRequest {
  type: 'spawn'
  id: number
  sessionId: string
  projectId: string
  initial: SessionStatus
  file: string
  args: string[]
  env: Record<string, string>
  cwd: string
  cols: number
  rows: number
}

export type ClientMessage =
  | { type: 'hello'; protocol: number }
  | SpawnRequest
  | { type: 'write'; sessionId: string; data: string }
  | { type: 'resize'; sessionId: string; cols: number; rows: number }
  | { type: 'kill'; sessionId: string }
  /** Stops every session and the host itself. */
  | { type: 'shutdown' }
  /** The app's answer to a relayed MCP request. */
  | { type: 'mcp-reply'; id: number; reply: unknown }

export type HostMessage =
  | {
      type: 'welcome'
      protocol: number
      hookPort: number
      hookToken: string
      sessions: HostSession[]
      /**
       * The host relays the board tools' MCP requests to the app. An older
       * host doesn't, which needs no new protocol: its sessions go without.
       */
      mcp?: boolean
      /**
       * The hook server takes the token in an Authorization header, so hook
       * commands can keep it out of their command line, which any user on
       * the Mac can list. An older host only reads it from the URL.
       */
      hookAuthHeader?: boolean
      /**
       * On Windows, the key the host wrote to the host folder, which only the
       * user can read. It shows the app this pipe is its own host's
       * (trustsHost), as anyone on the PC can make a pipe with that name.
       */
      key?: string
    }
  | { type: 'spawned'; id: number; error?: string }
  | { type: 'data'; sessionId: string; data: string }
  | { type: 'exit'; sessionId: string; exitCode: number }
  | { type: 'hook'; sessionId: string; event: string; payload: Record<string, unknown> }
  /** A JSON-RPC request a session's claude process sent to the board tools, for the app to answer. */
  | { type: 'mcp'; id: number; sessionId: string; message: unknown }

/**
 * Where a claude process reaches the board tools: on the hook server, with the
 * hook token as its bearer token and its session id in this header.
 */
export const MCP_PATH = '/mcp'
export const MCP_SESSION_HEADER = 'X-Nodus-Session'

/** Keystrokes the status heuristics act on (noteUserInput in hooks.ts). */
export const STATUS_KEY = /^(?:\r|\x1b|\x03|[1-9])$/

export interface HostPaths {
  /** Private to the user, like the socket inside it: whoever can connect can run commands. */
  dir: string
  socket: string
  log: string
  /** The curl config that gives hook commands the token (writeHookConfig). */
  hookConfig: string
  /** On Windows, the host's key, which its welcome has to carry (trustsHost). */
  key: string
}

export function hostPaths(userData: string, platform: NodeJS.Platform = process.platform): HostPaths {
  const dir = path.join(userData, 'host')
  const hash = () => crypto.createHash('sha1').update(dir).digest('hex').slice(0, 12)
  let socket = path.join(dir, 'host.sock')
  // Windows has no sockets in folders. A pipe's default security lets only
  // its owner (and admins) write to it, so other users can't say hello. But
  // anyone can make a pipe of this name first, so the host proves itself with
  // its key.
  if (platform === 'win32') socket = `\\\\.\\pipe\\nodus-${hash()}`
  // macOS caps socket paths at 104 bytes; the per-user temp folder is private too.
  else if (Buffer.byteLength(socket) > 100) socket = path.join(os.tmpdir(), `nodus-${hash()}.sock`)
  return {
    dir,
    socket,
    log: path.join(dir, 'host.log'),
    hookConfig: path.join(dir, 'hook.curlrc'),
    key: path.join(dir, 'host.key')
  }
}

/**
 * Joins a program's arguments into a Windows command line, quoted the way
 * programs built with the Microsoft C runtime split it back up; node-pty puts
 * the program in front. Its own joiner leaves an argument that starts and
 * ends with a quote unquoted, so a prompt such as "fix the tests" would
 * arrive in pieces.
 */
export function windowsCommandLine(args: string[]): string {
  const quote = (arg: string) => {
    if (arg && !/[\s"]/.test(arg)) return arg
    // Backslashes only need doubling before a quote, including the closing one.
    return '"' + arg.replace(/(\\*)"/g, '$1$1\\"').replace(/(\\+)$/, '$1$1') + '"'
  }
  return args.map(quote).join(' ')
}

export function ensurePrivateDir(dir: string) {
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 })
  fs.chmodSync(dir, 0o700)
}

/** Writes a file in a private folder that only the user can read, beside it and renamed into place. */
export function writePrivateFile(file: string, text: string) {
  ensurePrivateDir(path.dirname(file))
  const tmp = `${file}.${process.pid}.tmp`
  fs.writeFileSync(tmp, text, { mode: 0o600 })
  fs.renameSync(tmp, file)
}

/**
 * Writes the curl config hook commands read the host's token from. Only the
 * user can read it. The token goes inside quotes in a config file, where a
 * quote or a new line would add options of its own, so only plain ones do.
 */
export function writeHookConfig(paths: HostPaths, token: string) {
  if (!/^[\w-]+$/.test(token)) throw new Error('The session host’s token can’t go in a curl config')
  writePrivateFile(paths.hookConfig, `header = "Authorization: Bearer ${token}"\n`)
}

/**
 * Whether a host's welcome shows it's one this app's user started. On Windows
 * it has to carry the key in the host folder, which only the user can read:
 * anyone on the PC can make a pipe with the host's name before the host does,
 * and would then be handed every session. A Mac's socket sits in a folder only
 * the user can open, so it needs no key.
 */
export function trustsHost(key: unknown, paths: HostPaths, platform: NodeJS.Platform = process.platform): boolean {
  if (platform !== 'win32') return true
  if (typeof key !== 'string' || !key) return false
  try {
    return fs.readFileSync(paths.key, 'utf8') === key
  } catch {
    return false
  }
}

export const encode = (msg: ClientMessage | HostMessage) => JSON.stringify(msg) + '\n'

/** Splits a socket's bytes into messages. Malformed lines are dropped. */
export function messageReader<T>(onMessage: (msg: T) => void): (chunk: Buffer) => void {
  const decoder = new StringDecoder('utf8')
  let partial = ''
  return (chunk) => {
    const text = decoder.write(chunk)
    // Only the new text is scanned, so a long message arriving in pieces stays linear.
    let start = 0
    let nl: number
    while ((nl = text.indexOf('\n', start)) >= 0) {
      const line = partial + text.slice(start, nl)
      partial = ''
      start = nl + 1
      if (!line) continue
      let msg: T
      try {
        msg = JSON.parse(line)
      } catch {
        continue
      }
      onMessage(msg)
    }
    partial += text.slice(start)
  }
}
