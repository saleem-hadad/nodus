import crypto from 'crypto'
import fs from 'fs'
import os from 'os'
import path from 'path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { encode, ensurePrivateDir, hostPaths, messageReader, windowsCommandLine, writeHookConfig } from './hostProtocol'

const isWindows = process.platform === 'win32'

const read = (chunks: Buffer[]) => {
  const messages: unknown[] = []
  const feed = messageReader((msg) => messages.push(msg))
  for (const chunk of chunks) feed(chunk)
  return messages
}

describe('messageReader', () => {
  it('reads several messages from one chunk', () => {
    const bytes = Buffer.from(encode({ type: 'kill', sessionId: 'a' }) + encode({ type: 'shutdown' }))
    expect(read([bytes])).toEqual([{ type: 'kill', sessionId: 'a' }, { type: 'shutdown' }])
  })

  it('waits for the rest of a message split across chunks', () => {
    const bytes = Buffer.from(encode({ type: 'write', sessionId: 'a', data: 'hello world' }))
    const messages: unknown[] = []
    const feed = messageReader((msg) => messages.push(msg))
    feed(bytes.subarray(0, 10))
    expect(messages).toEqual([])
    feed(bytes.subarray(10))
    expect(messages).toEqual([{ type: 'write', sessionId: 'a', data: 'hello world' }])
  })

  it('keeps a character whose bytes are split across chunks', () => {
    const bytes = Buffer.from(encode({ type: 'write', sessionId: 'a', data: 'héllo 🎉' }))
    // Every split point, including inside the two- and four-byte characters.
    for (let i = 1; i < bytes.length; i++) {
      expect(read([bytes.subarray(0, i), bytes.subarray(i)])).toEqual([
        { type: 'write', sessionId: 'a', data: 'héllo 🎉' }
      ])
    }
  })

  it('drops malformed and blank lines and carries on', () => {
    const bytes = Buffer.from('not json\n\n' + encode({ type: 'shutdown' }))
    expect(read([bytes])).toEqual([{ type: 'shutdown' }])
  })

  it('keeps newlines inside a message', () => {
    const msg = { type: 'write', sessionId: 'a', data: 'line one\nline two\r\n' }
    expect(read([Buffer.from(encode(msg as never))])).toEqual([msg])
  })
})

describe('host files', () => {
  let dir: string

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-host-'))
  })

  afterEach(() => {
    fs.rmSync(dir, { recursive: true, force: true })
  })

  it('keeps the socket in the host folder when the path is short enough', () => {
    const paths = hostPaths(dir, 'darwin')
    expect(paths).toEqual({
      dir: path.join(dir, 'host'),
      socket: path.join(dir, 'host', 'host.sock'),
      log: path.join(dir, 'host', 'host.log'),
      hookConfig: path.join(dir, 'host', 'hook.curlrc')
    })
  })

  it('moves a socket with a long path to the temp folder, under a name from the folder', () => {
    const userData = path.join(dir, 'x'.repeat(120))
    const hostDir = path.join(userData, 'host')
    const hash = crypto.createHash('sha1').update(hostDir).digest('hex').slice(0, 12)
    expect(hostPaths(userData, 'darwin').socket).toBe(path.join(os.tmpdir(), `nodus-${hash}.sock`))
    expect(hostPaths(userData, 'darwin').hookConfig).toBe(path.join(hostDir, 'hook.curlrc'))
  })

  it('names a pipe on Windows, from the folder too', () => {
    const userData = path.join(dir, 'Nodus')
    const hash = crypto.createHash('sha1').update(path.join(userData, 'host')).digest('hex').slice(0, 12)
    expect(hostPaths(userData, 'win32').socket).toBe(`\\\\.\\pipe\\nodus-${hash}`)
    expect(hostPaths(userData, 'win32').hookConfig).toBe(path.join(userData, 'host', 'hook.curlrc'))
  })

  it.skipIf(isWindows)('makes the host folder private, even one that exists already', () => {
    const hostDir = path.join(dir, 'host')
    fs.mkdirSync(hostDir, { mode: 0o755 })
    fs.chmodSync(hostDir, 0o755)
    ensurePrivateDir(hostDir)
    expect(fs.statSync(hostDir).mode & 0o777).toBe(0o700)
  })

  it('writes the hook config readable only by the user', () => {
    const paths = hostPaths(dir)
    writeHookConfig(paths, 'first')
    writeHookConfig(paths, 'second')
    expect(fs.readFileSync(paths.hookConfig, 'utf8')).toBe('header = "Authorization: Bearer second"\n')
    // Windows has no mode bits; the folder's ACL is the user's.
    if (!isWindows) {
      expect(fs.statSync(paths.hookConfig).mode & 0o777).toBe(0o600)
      expect(fs.statSync(paths.dir).mode & 0o777).toBe(0o700)
    }
    // Written beside it and renamed into place.
    expect(fs.readdirSync(paths.dir)).toEqual(['hook.curlrc'])
  })
})

/** Splits a command line back up the way the Microsoft C runtime does. */
function splitCommandLine(line: string): string[] {
  const args: string[] = []
  let arg: string | null = null
  let quoted = false
  for (let i = 0; i < line.length; i++) {
    const c = line[i]
    if (c === '\\') {
      let n = 0
      while (line[i] === '\\') {
        n++
        i++
      }
      arg ??= ''
      if (line[i] === '"') {
        // 2n backslashes and a quote: n backslashes, and the quote opens or closes; 2n+1: a literal quote.
        arg += '\\'.repeat(n >> 1)
        if (n % 2) arg += '"'
        else quoted = !quoted
      } else {
        arg += '\\'.repeat(n)
        i--
      }
    } else if (c === '"') {
      arg ??= ''
      quoted = !quoted
    } else if ((c === ' ' || c === '\t') && !quoted) {
      if (arg !== null) args.push(arg)
      arg = null
    } else {
      arg = (arg ?? '') + c
    }
  }
  if (arg !== null) args.push(arg)
  return args
}

describe('windowsCommandLine', () => {
  it('quotes only what needs it', () => {
    expect(windowsCommandLine(['--resume', 'abc', 'two words', ''])).toBe('--resume abc "two words" ""')
  })

  it('comes back as the same arguments, whatever they hold', () => {
    const args = [
      '--settings',
      JSON.stringify({ hooks: { Stop: [{ command: 'curl -K "$NODUS_HOOK_CONFIG" >/dev/null 2>&1; exit 0' }] } }),
      '--',
      '"fix the tests"',
      'C:\\Program Files\\Nodus\\',
      'ends in two \\\\',
      'a\\"b',
      'tab\there',
      'line one\nline two',
      '',
      'plain'
    ]
    expect(splitCommandLine(windowsCommandLine(args))).toEqual(args)
  })
})
