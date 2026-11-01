/** What the bar makes besides a session, which is what it makes unless told otherwise. */
export type Target = 'view' | 'note'

export interface SlashCommand {
  target: Target
  command: string
  description: string
}

export const COMMANDS: SlashCommand[] = [
  { target: 'view', command: '/view', description: 'A live page Claude keeps up to date' },
  { target: 'note', command: '/note', description: 'A sticky on the board, nothing runs' }
]

/** The commands what's typed could still become: while it's one word that starts with a slash. */
export function matchCommands(text: string): SlashCommand[] {
  if (!/^\/\S*$/.test(text)) return []
  const typed = text.toLowerCase()
  return COMMANDS.filter((c) => c.command.startsWith(typed))
}

/** A command typed out and followed by a space, or pasted with more after it: what it makes, and the rest. */
export function takeCommand(text: string): { target: Target; rest: string } | null {
  const m = /^\/(view|note)\s+([\s\S]*)$/i.exec(text)
  return m ? { target: m[1].toLowerCase() as Target, rest: m[2] } : null
}

/** Something selected on the board, which a new session gets to read first. */
export type ContextItem =
  | { kind: 'session'; title: string; branch?: string; lastReply?: string; transcript?: string }
  | { kind: 'note'; text: string }
  | { kind: 'view'; title: string; description: string }

const quote = (text: string) =>
  text
    .trim()
    .split('\n')
    .map((line) => (line ? `> ${line}` : '>'))
    .join('\n')

function describe(item: ContextItem): string {
  switch (item.kind) {
    case 'session': {
      const lines = [`## Session “${item.title}”`]
      if (item.branch) lines.push(`Branch: ${item.branch}`)
      lines.push(item.lastReply?.trim() ? `Its last reply:\n${quote(item.lastReply)}` : 'It has no replies yet.')
      if (item.transcript) lines.push(`Its whole conversation, if you need more: ${item.transcript}`)
      return lines.join('\n')
    }
    case 'note':
      return `## Note\n${item.text.trim()}`
    case 'view':
      return `## View “${item.title}”\n${item.description.trim()}`
  }
}

/** The prompt, after what was selected on the board, written out for Claude to read first. */
export function withContext(items: ContextItem[], prompt: string): string {
  if (items.length === 0) return prompt
  const head = 'For context, I selected these on my board:'
  return `${head}\n\n${items.map(describe).join('\n\n')}\n\n---\n\n${prompt}`
}
