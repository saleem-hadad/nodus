import { describe, test } from 'node:test';
import assert from 'node:assert/strict';
import {
  buildBody,
  buildTitle,
  charCount,
  cleanMeta,
  isValidUsername,
  neutralizeMentions,
} from '../issue.js';
import { sniffImage } from '../app.js';

const WJ = '⁠';

describe('neutralizeMentions', () => {
  test('breaks mentions with a word joiner', () => {
    assert.equal(neutralizeMentions('@octocat'), `@${WJ}octocat`);
    assert.equal(neutralizeMentions('hi @octocat, ping (@hubot)'), `hi @${WJ}octocat, ping (@${WJ}hubot)`);
    assert.equal(neutralizeMentions('line\n@octo-cat'), `line\n@${WJ}octo-cat`);
    assert.equal(neutralizeMentions('cc @org/team'), `cc @${WJ}org/team`);
    assert.equal(neutralizeMentions('ask.@octocat'), `ask.@${WJ}octocat`);
  });

  test('breaks mentions GitHub only sees once the markdown is rendered', () => {
    assert.equal(neutralizeMentions('_@octocat_ and __@hubot__'), `_@${WJ}octocat_ and __@${WJ}hubot__`);
    for (const at of ['&#64;', '&#064;', '&#x40;', '&#X0040;', '&commat;']) {
      assert.equal(neutralizeMentions(`hi ${at}octocat`), `hi ${at}${WJ}octocat`);
    }
    assert.equal(neutralizeMentions('www.example.com@octocat'), `www.example.com@${WJ}octocat`);
    assert.equal(neutralizeMentions('name_@octocat'), `name_@${WJ}octocat`);
  });

  test('leaves emails and lone @ signs alone', () => {
    assert.equal(neutralizeMentions('mail a@b.com or me_x@example.org'), 'mail a@b.com or me_x@example.org');
    assert.equal(neutralizeMentions('5 @ 10, @ home, @-dash'), '5 @ 10, @ home, @-dash');
  });

  test('is idempotent', () => {
    const once = neutralizeMentions('hey @octocat, _@hubot_, &#64;ada');
    assert.equal(neutralizeMentions(once), once);
  });
});

describe('isValidUsername', () => {
  test('accepts GitHub usernames', () => {
    for (const name of ['octocat', 'a', 'octo-cat', 'A1-b2-C3', 'x'.repeat(39)]) {
      assert.ok(isValidUsername(name), name);
    }
  });

  test('rejects everything else', () => {
    for (const name of ['', '-octocat', 'octocat-', 'octo--cat', 'octo_cat', '@octocat', 'x'.repeat(40), 'a b', undefined]) {
      assert.ok(!isValidUsername(name), String(name));
    }
  });
});

describe('cleanMeta', () => {
  test('keeps a safe alphabet and 40 characters at most', () => {
    assert.equal(cleanMeta('macOS 27.2'), 'macOS 27.2');
    assert.equal(cleanMeta('1.0.2-beta (42)'), '1.0.2-beta (42)');
    assert.equal(cleanMeta('<b>@root</b>\n arm64'), 'brootb arm64');
    assert.equal(cleanMeta('x'.repeat(50)), 'x'.repeat(40));
    assert.equal(cleanMeta(undefined), '');
    assert.equal(cleanMeta(['1.0.2']), '');
  });
});

describe('charCount', () => {
  test('counts characters, not UTF-16 units', () => {
    assert.equal(charCount('😀😀'), 2);
  });
});

describe('buildTitle', () => {
  test('collapses whitespace and neutralizes mentions', () => {
    assert.equal(buildTitle('  Board\n froze\t\twhen @bob pinched  '), `Board froze when @${WJ}bob pinched`);
  });

  test("stays within GitHub's 256-character limit", () => {
    const title = '@a '.repeat(85) + 'x'; // 256 characters before neutralizing
    assert.equal(charCount(buildTitle(title)), 256);
  });
});

describe('buildBody', () => {
  const SOURCE = '<sub>Sent from Nodus feedback · 1.0.2 · macOS 27.2 · arm64</sub>';
  const META = { version: '1.0.2', os: 'macOS 27.2', arch: 'arm64' };
  const PLAIN = '<sub>Sent from Nodus feedback</sub>\n\n';

  test('puts the source line above a plain body', () => {
    assert.equal(buildBody({ body: 'It froze.', ...META }), `${SOURCE}\n\nIt froze.`);
  });

  test('keeps the source line above whatever the body opens and never closes', () => {
    for (const trap of ['<!--', "<details title='x"]) {
      assert.ok(buildBody({ body: `It froze.\n\n${trap}`, ...META }).startsWith(`${SOURCE}\n\n`), trap);
    }
  });

  test('lists screenshots under the body', () => {
    const body = buildBody({
      body: 'It froze.',
      screenshotUrls: ['https://github.com/user-attachments/assets/a', 'https://github.com/user-attachments/assets/b'],
      ...META,
    });
    assert.equal(
      body,
      `${SOURCE}\n\n` +
        'It froze.\n\n' +
        '![Screenshot 1](https://github.com/user-attachments/assets/a)\n\n' +
        '![Screenshot 2](https://github.com/user-attachments/assets/b)',
    );
  });

  test('leaves out missing parts of the source line', () => {
    assert.equal(buildBody({ body: 'x', os: 'Windows 11' }), '<sub>Sent from Nodus feedback · Windows 11</sub>\n\nx');
    assert.equal(buildBody({ body: 'x' }), `${PLAIN}x`);
  });

  test("keeps the reporter's own line live and neutralizes the rest", () => {
    const body = buildBody({ body: 'Reported by @OctoCat\nIt froze. cc @hubot', reporter: 'octocat' });
    assert.equal(body, `${PLAIN}Reported by @OctoCat\nIt froze. cc @${WJ}hubot`);
  });

  test('handles a body that is only the reporter line, and CRLF', () => {
    assert.equal(buildBody({ body: 'Reported by @octocat', reporter: 'octocat' }), `${PLAIN}Reported by @octocat`);
    assert.equal(buildBody({ body: 'Reported by @octocat\r\nhi @x', reporter: 'octocat' }), `${PLAIN}Reported by @octocat\r\nhi @${WJ}x`);
  });

  test('neutralizes the reporter line when it is not exactly the reporter', () => {
    const cases = [
      { body: 'Reported by @someone-else\nhi', reporter: 'octocat' },
      { body: 'Reported by @octocat\nhi', reporter: '' },
      { body: 'Reported by @octocat and @hubot\nhi', reporter: 'octocat' },
      { body: 'reported by @octocat\nhi', reporter: 'octocat' },
      { body: 'hi\nReported by @octocat', reporter: 'octocat' },
    ];
    for (const c of cases) {
      assert.ok(!/@[A-Za-z0-9]/.test(buildBody(c)), JSON.stringify(c));
    }
  });

  test('leaves emails untouched', () => {
    assert.equal(buildBody({ body: 'Write to a@b.com' }), `${PLAIN}Write to a@b.com`);
  });
});

describe('sniffImage', () => {
  test('detects the four allowed types from magic bytes', () => {
    const png = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0]);
    const jpeg = Buffer.from([0xff, 0xd8, 0xff, 0xe0, 0]);
    const gif = Buffer.from('GIF89a......');
    const webp = Buffer.from('RIFF\x10\x00\x00\x00WEBPVP8 ', 'latin1');
    assert.deepEqual(sniffImage(png), { mime: 'image/png', ext: 'png' });
    assert.deepEqual(sniffImage(jpeg), { mime: 'image/jpeg', ext: 'jpg' });
    assert.deepEqual(sniffImage(gif), { mime: 'image/gif', ext: 'gif' });
    assert.deepEqual(sniffImage(webp), { mime: 'image/webp', ext: 'webp' });
  });

  test('rejects anything else', () => {
    assert.equal(sniffImage(Buffer.from('just some text')), null);
    assert.equal(sniffImage(Buffer.alloc(0)), null);
    assert.equal(sniffImage(Buffer.from('RIFF\x10\x00\x00\x00WAVE', 'latin1')), null);
  });
});
