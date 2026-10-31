import crypto from 'crypto'
import fs from 'fs'
import os from 'os'
import path from 'path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { encode, ensurePrivateDir, hostPaths, messageReader, writeHookConfig } from './hostProtocol'

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
    const paths = hostPaths(dir)
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
    expect(hostPaths(userData).socket).toBe(path.join(os.tmpdir(), `nodus-${hash}.sock`))
    expect(hostPaths(userData).hookConfig).toBe(path.join(hostDir, 'hook.curlrc'))
  })

  it('makes the host folder private, even one that exists already', () => {
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
    expect(fs.statSync(paths.hookConfig).mode & 0o777).toBe(0o600)
    expect(fs.statSync(paths.dir).mode & 0o777).toBe(0o700)
    // Written beside it and renamed into place.
    expect(fs.readdirSync(paths.dir)).toEqual(['hook.curlrc'])
  })
})
