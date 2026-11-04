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
