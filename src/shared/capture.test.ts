import { describe, expect, it } from 'vitest'
import { describeAttachments, titleFrom, withScreen } from './capture'

describe('titleFrom', () => {
  it('takes the first sentence', () => {
    expect(titleFrom('Refunds fail with a coupon. Write a test, then fix it')).toBe('Refunds fail with a coupon')
  })

  it('takes the first line', () => {
    expect(titleFrom('Bump Stripe\nand run the tests')).toBe('Bump Stripe')
  })

  it('cuts a long sentence at a word', () => {
    const title = titleFrom('Look into why the checkout total is rounded down when the customer has a coupon applied')
    expect(title.length).toBeLessThanOrEqual(48)
    expect(title).toBe('Look into why the checkout total is rounded…')
  })

  it("doesn't take a dot inside a word as the end of a sentence", () => {
    expect(titleFrom('Fix README.md links')).toBe('Fix README.md links')
  })
})

describe('describeAttachments', () => {
  it('is empty with nothing attached', () => {
    expect(describeAttachments([])).toBe('')
    expect(describeAttachments([{ kind: 'text', text: '   ' }])).toBe('')
  })

  it('lists the tab, images and files, then the copied text', () => {
    const out = describeAttachments([
      { kind: 'text', text: 'TypeError: amount.minus' },
      { kind: 'tab', app: 'Safari', title: 'Refund fails #482', url: 'https://github.com/acme/storefront/issues/482' },
      { kind: 'image', path: '/tmp/shot.png', thumb: '' },
      { kind: 'image', path: '/tmp/copied.png', thumb: '', copied: true },
      { kind: 'file', path: '/logs/checkout.log', name: 'checkout.log', size: 12 }
    ])
    expect(out).toBe(
      [
        'From my screen as I wrote this:',
        '',
        '- The Safari tab I had open: Refund fails #482 (https://github.com/acme/storefront/issues/482)',
        '- A screenshot: /tmp/shot.png',
        '- An image I copied: /tmp/copied.png',
        '- A file: /logs/checkout.log',
        '',
        'Text I copied:',
        '```',
        'TypeError: amount.minus',
        '```'
      ].join('\n')
    )
  })

  it('fences copied text with more backticks than it holds', () => {
    const out = describeAttachments([{ kind: 'text', text: '```js\nx()\n```' }])
    expect(out).toContain('````\n```js\nx()\n```\n````')
  })
})

describe('withScreen', () => {
  it('leaves text alone with nothing attached', () => {
    expect(withScreen('  Fix it  ', [])).toBe('Fix it')
  })

  it('puts the screen after the text', () => {
    expect(withScreen('Fix it', [{ kind: 'file', path: '/a.log', name: 'a.log', size: 1 }])).toBe(
      'Fix it\n\nFrom my screen as I wrote this:\n\n- A file: /a.log'
    )
  })
})
