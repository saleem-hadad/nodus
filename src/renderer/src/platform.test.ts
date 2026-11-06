import { afterEach, describe, expect, it, vi } from 'vitest'

/** platform.ts as it loads on a machine whose navigator says `name`. */
async function onPlatform(name: string) {
  vi.resetModules()
  vi.stubGlobal('navigator', { platform: name })
  return import('./platform')
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('keys', () => {
  it('writes a Mac shortcut as Windows does', async () => {
    const { keys, platform } = await onPlatform('Win32')
    expect(platform).toBe('windows')
    expect(keys('⇧⌘F')).toBe('Ctrl+Shift+F')
    expect(keys('⌘↩')).toBe('Ctrl+Enter')
    expect(keys('⌘')).toBe('Ctrl')
    expect(keys('⌫')).toBe('Del')
    expect(keys('↩')).toBe('Enter')
  })

  it('leaves it be on a Mac', async () => {
    const { keys, platform } = await onPlatform('MacIntel')
    expect(platform).toBe('mac')
    expect(keys('⇧⌘F')).toBe('⇧⌘F')
  })
})

describe('modKey', () => {
  const press = { metaKey: true, ctrlKey: false }

  it('is ⌘ on a Mac and Ctrl elsewhere', async () => {
    expect((await onPlatform('MacIntel')).modKey(press)).toBe(true)
    const windows = await onPlatform('Win32')
    expect(windows.modKey(press)).toBe(false)
    expect(windows.modKey({ metaKey: false, ctrlKey: true })).toBe(true)
    expect(windows.otherModKey(press)).toBe(true)
  })
})

describe('shortcutKey', () => {
  it('reads a Latin layout’s letter as it is', async () => {
    const { shortcutKey } = await onPlatform('Win32')
    expect(shortcutKey({ key: 'c', keyCode: 67 })).toBe('c')
    expect(shortcutKey({ key: 'W', keyCode: 87 })).toBe('w')
    // AZERTY's A sits where QWERTY has Q, and is still A.
    expect(shortcutKey({ key: 'a', keyCode: 65 })).toBe('a')
  })

  it('reads the letter from the key code on another alphabet’s layout', async () => {
    const { shortcutKey } = await onPlatform('Win32')
    // Arabic and Russian: Ctrl+V and Ctrl+C.
    expect(shortcutKey({ key: 'ر', keyCode: 86 })).toBe('v')
    expect(shortcutKey({ key: 'с', keyCode: 67 })).toBe('c')
  })

  it('leaves keys that aren’t letters be', async () => {
    const { shortcutKey } = await onPlatform('Win32')
    expect(shortcutKey({ key: 'Enter', keyCode: 13 })).toBe('enter')
    expect(shortcutKey({ key: 'é', keyCode: 50 })).toBe('é')
  })
})
