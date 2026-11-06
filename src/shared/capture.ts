// Floating capture: how what the user typed, and what they brought from their
// screen, is written out for Claude and for the board.
import type { CaptureAttachment, CaptureKind } from './types'

/** Tab cycles through these, in this order. */
export const CAPTURE_KINDS: readonly CaptureKind[] = ['session', 'into', 'note', 'view']

export const isCaptureKind = (value: unknown): value is CaptureKind => CAPTURE_KINDS.includes(value as CaptureKind)

/** How much copied text goes along with a capture. */
export const MAX_COPIED_CHARS = 20_000

/** A short name for a capture: its first sentence, or as much of it as fits. */
export function titleFrom(text: string, max = 48): string {
  const line = text.trim().split('\n')[0] ?? ''
  const sentence = (/^(.+?[.!?])(\s|$)/.exec(line)?.[1] ?? line).replace(/[.!?]+$/, '').trim()
  if (sentence.length <= max) return sentence
  const cut = sentence.slice(0, max - 1)
  const word = cut.lastIndexOf(' ')
  return `${(word > max / 2 ? cut.slice(0, word) : cut).trimEnd()}…`
}

const fence = (text: string) => {
  // A fence longer than any run of backticks in the text, so it can't close early.
  const longest = Math.max(2, ...[...text.matchAll(/`+/g)].map((m) => m[0].length))
  const ticks = '`'.repeat(longest + 1)
  return `${ticks}\n${text}\n${ticks}`
}

/**
 * What came from the screen, as lines for Claude to read: the browser tab,
 * screenshots and files by path (Claude reads them itself), and copied text
 * in full. Empty when nothing came along.
 */
export function describeAttachments(attachments: CaptureAttachment[]): string {
  const lines: string[] = []
  const copied: string[] = []
  for (const a of attachments) {
    switch (a.kind) {
      case 'tab':
        lines.push(`- The ${a.app} tab I had open: ${a.title ? `${a.title} (${a.url})` : a.url}`)
        break
      case 'image':
        lines.push(`- ${a.copied ? 'An image I copied' : 'A screenshot'}: ${a.path}`)
        break
      case 'file':
        lines.push(`- A file: ${a.path}`)
        break
      case 'text': {
        const text = a.text.trim().slice(0, MAX_COPIED_CHARS)
        if (text) copied.push(text)
        break
      }
    }
  }
  if (!lines.length && !copied.length) return ''
  const parts = ['From my screen as I wrote this:']
  if (lines.length) parts.push(lines.join('\n'))
  for (const text of copied) parts.push(`Text I copied:\n${fence(text)}`)
  return parts.join('\n\n')
}

/** The text, followed by what came from the screen. */
export function withScreen(text: string, attachments: CaptureAttachment[]): string {
  const screen = describeAttachments(attachments)
  return screen ? `${text.trim()}\n\n${screen}` : text.trim()
}
