// usage: node terminal.mjs <harness dir> <world dir> ids
//        node terminal.mjs <harness dir> <world dir> status <session key> working|waiting|done [agent key]
//        node terminal.mjs <harness dir> <world dir> usage <session key>
// What the parity harness's `liven`/`status`/`usage` do to the Rust app's
// session host: the seeded world's ids, and Claude Code's hooks posted to
// the host's hook endpoint (its port and token come from the host's
// welcome, never written anywhere). Stand-ins only; no real claude runs.
import crypto from 'node:crypto'
import fs from 'node:fs'
import http from 'node:http'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'

const [harness, work, command, ...args] = process.argv.slice(2)
const world = await import(path.join(path.resolve(harness), 'seed', 'world.mjs'))
const { agentIdOf } = await import(path.join(path.resolve(harness), 'seed', 'index.mjs'))

/** The host's socket, as `lib/app.mjs` finds it. */
function hostSocket(userData) {
  const dir = path.join(userData, 'host')
  const socket = path.join(dir, 'host.sock')
  if (Buffer.byteLength(socket) <= 100) return socket
  const hash = crypto.createHash('sha1').update(dir).digest('hex').slice(0, 12)
  return path.join(os.tmpdir(), `nodus-${hash}.sock`)
}

async function endpoint() {
  const socketPath = hostSocket(path.join(path.resolve(work), 'userData'))
  for (let i = 0; i < 100 && !fs.existsSync(socketPath); i++) await new Promise((r) => setTimeout(r, 100))
  return new Promise((resolve, reject) => {
    const socket = net.connect(socketPath)
    let buf = ''
    socket.on('data', (d) => {
      buf += d.toString('utf8')
      const nl = buf.indexOf('\n')
      if (nl < 0) return
      const msg = JSON.parse(buf.slice(0, nl))
      socket.destroy()
      resolve({ port: msg.hookPort, token: msg.hookToken })
    })
    socket.on('error', reject)
    socket.write(JSON.stringify({ type: 'hello', protocol: 0 }) + '\n')
  })
}

async function hook(key, event, payload = {}) {
  const { port, token } = await endpoint()
  const body = JSON.stringify(payload)
  await new Promise((resolve, reject) => {
    const req = http.request(
      {
        host: '127.0.0.1',
        port,
        method: 'POST',
        path: `/hook?evt=${event}&sid=${world.sessionId(key)}`,
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(body) }
      },
      (res) => {
        res.resume()
        res.on('end', () => (res.statusCode < 300 ? resolve() : reject(new Error(`Hook ${event}: HTTP ${res.statusCode}`))))
      }
    )
    req.on('error', reject)
    req.end(body)
  })
}

if (command === 'ids') {
  const keys = ['storefront', 'docs-site', 'infra', 'mobile-app']
  const sessions = ['sf-skeleton', 'sf-tax', 'sf-cart', 'sf-terminal', 'sf-arch-01', 'in-certs', 'in-shell', 'in-alerts']
  console.log(
    JSON.stringify({
      projects: Object.fromEntries(keys.map((k) => [k, world.projectId(k)])),
      sessions: Object.fromEntries(sessions.map((k) => [k, world.sessionId(k)])),
      agents: Object.fromEntries(['sf-a1', 'sf-a2', 'sf-a3'].map((k) => [k, agentIdOf(k)]))
    })
  )
} else if (command === 'status') {
  const [key, status, agent] = args
  const payload = agent ? { agent_id: agentIdOf(agent) } : {}
  if (status === 'working') await hook(key, agent ? 'SubagentStart' : 'UserPromptSubmit', payload)
  else if (status === 'waiting') await hook(key, 'PermissionRequest', payload)
  else await hook(key, agent ? 'SubagentStop' : 'Stop', payload)
} else if (command === 'usage') {
  await hook(args[0], 'StatusLine', world.USAGE[args[0]])
} else {
  console.error(`unknown command ${command}`)
  process.exit(1)
}
