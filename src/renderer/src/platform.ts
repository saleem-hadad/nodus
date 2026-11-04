// Shortcuts take ⌘ on a Mac and Ctrl elsewhere, and are written the way each
// platform writes them.

export const isMac = /^Mac/.test(navigator.platform)

/** Which platform's look the window has (data-platform on <html>, for CSS). */
export const platform = isMac ? 'mac' : /^Win/.test(navigator.platform) ? 'windows' : 'linux'

type Modifiers = { metaKey: boolean; ctrlKey: boolean }

/** Whether the shortcut key is held: ⌘ on a Mac, Ctrl elsewhere. */
export const modKey = (e: Modifiers) => (isMac ? e.metaKey : e.ctrlKey)

/** Whether the other of ⌘ and Ctrl is held, which a shortcut mustn't have. */
export const otherModKey = (e: Modifiers) => (isMac ? e.ctrlKey : e.metaKey)

const NAMES: Record<string, string> = { '⌘': 'Ctrl', '⌃': 'Ctrl', '⌥': 'Alt', '⇧': 'Shift', '↩': 'Enter', '⌫': 'Del' }
const MODIFIERS = ['Ctrl', 'Alt', 'Shift']

/** A shortcut as a Mac writes it (⇧⌘F, ⌘↩, ⌫), as this platform writes it: Ctrl+Shift+F, Ctrl+Enter, Del. */
export function keys(mac: string): string {
  if (isMac) return mac
  const parts = [...mac].map((c) => NAMES[c] ?? c)
  const key = parts.filter((p) => !MODIFIERS.includes(p)).join('')
  return [...MODIFIERS.filter((m) => parts.includes(m)), ...(key ? [key] : [])].join('+')
}
