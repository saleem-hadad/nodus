// A stand-in for a terminal card's login shell in the parity harness: a fixed
// screen (a test run and a prompt) instead of the user's real shell, whose
// prompt would show their host name and history. Like the claude stand-in it
// hides the hardware cursor and draws its own, so no blink phase shows up in a
// screenshot, and it redraws whole on every resize.
import fs from 'node:fs'
import path from 'node:path'

const ESC = '\x1b['
const green = (s) => `${ESC}32m${s}${ESC}39m`
const cyan = (s) => `${ESC}36m${s}${ESC}39m`
const blue = (s) => `${ESC}34m${s}${ESC}39m`
const red = (s) => `${ESC}31m${s}${ESC}39m`
const yellow = (s) => `${ESC}33m${s}${ESC}39m`
const dim = (s) => `${ESC}2m${s}${ESC}22m`

function branch() {
  try {
    const head = fs.readFileSync(path.join(process.cwd(), '.git', 'HEAD'), 'utf8').trim()
    return head.startsWith('ref: refs/heads/') ? head.slice('ref: refs/heads/'.length) : head.slice(0, 7)
  } catch {
    return null
  }
}

function draw() {
  const name = path.basename(process.cwd())
  const b = branch()
  const prompt = `${green('➜')}  ${cyan(name)}${b ? ` ${blue('git:(')}${red(b)}${blue(')')} ${yellow('✗')}` : ''} `
  const rows = [
    `${prompt}pnpm test --run`,
    '',
    ` ${green('✓')} src/cart/total.test.ts ${dim('(12 tests)')} ${dim('18ms')}`,
    ` ${green('✓')} src/checkout/tax.test.ts ${dim('(8 tests)')} ${dim('9ms')}`,
    ` ${green('✓')} src/grid/ProductGrid.test.tsx ${dim('(5 tests)')} ${dim('41ms')}`,
    '',
    ` ${dim('Test Files')}  ${green('3 passed')} ${dim('(3)')}`,
    `      ${dim('Tests')}  ${green('25 passed')} ${dim('(25)')}`,
    '',
    `${prompt}${ESC}7m ${ESC}27m`
  ]
  process.stdout.write(`${ESC}?25l${ESC}H${ESC}2J${ESC}3J${rows.join('\r\n')}`)
}

if (process.stdin.isTTY) process.stdin.setRawMode(true)
process.stdin.resume()
process.stdin.on('data', () => {})
process.stdout.on('resize', draw)
draw()
