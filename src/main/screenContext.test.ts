import { describe, expect, it, vi } from 'vitest'

vi.mock('electron', () => ({
  app: {},
  clipboard: {},
  desktopCapturer: {},
  nativeImage: {},
  screen: {},
  shell: {},
  systemPreferences: {}
}))

import { isBrowser, parseAppInfo, tabScript } from './screenContext'

describe('parseAppInfo', () => {
  it('reads the name and bundle id from lsappinfo', () => {
    const out = [
      '"Safari" ASN:0x0-0x1a01a: (in front) ',
      '    bundleID="com.apple.Safari"',
      '    bundle path=[ NULL ] ',
      ' !cgsConnection !signalled type=[ NULL ]'
    ].join('\n')
    expect(parseAppInfo(out)).toEqual({ name: 'Safari', bundleId: 'com.apple.Safari' })
  })

  it('falls back to the bundle id when there is no name', () => {
    expect(parseAppInfo('[ NULL ] ASN:0x0-0x1:\n    bundleID="com.mitchellh.ghostty"')).toEqual({
      name: 'ghostty',
      bundleId: 'com.mitchellh.ghostty'
    })
  })

  it('has nothing without a bundle id', () => {
    expect(parseAppInfo('[ NULL ] ASN:0x0-0x1:\n    bundleID=[ NULL ]')).toBeNull()
  })
})

describe('tabScript', () => {
  it('asks Safari for its front document', () => {
    expect(tabScript('com.apple.Safari')).toBe(
      'tell application id "com.apple.Safari" to tell front document to return (URL as text) & linefeed & (name as text)'
    )
  })

  it('asks a Chromium browser for its active tab', () => {
    expect(tabScript('company.thebrowser.Browser')).toContain('tell active tab of front window')
  })

  it('has no script for apps that are not browsers it can read', () => {
    expect(tabScript('org.mozilla.firefox')).toBeNull()
    expect(isBrowser('com.microsoft.VSCode')).toBe(false)
  })

  it('never puts an odd bundle id into a script', () => {
    expect(tabScript('com.apple.Safari" to do shell script "x')).toBeNull()
  })
})
