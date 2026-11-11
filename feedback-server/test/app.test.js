// End-to-end tests: the real Express app on an ephemeral port, hit with real
// fetch + FormData, talking to a fake GitHub that records every call.

import { describe, test } from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import sharp from 'sharp';
import { createApp } from '../app.js';

const KEY = 'test-app-key';
const TOKEN = 'ghp_fake_token_for_tests_only';
const ENV = { GITHUB_TOKEN: TOKEN, NODUS_APP_KEY: KEY };
const WJ = '⁠';
const HOUR = 60 * 60 * 1000;

const blank = (width, height) => sharp({ create: { width, height, channels: 3, background: '#3366cc' } });
const PNG = await blank(40, 30).png().toBuffer();
const JPEG = await blank(40, 30).jpeg().withExif({ IFD0: { Artist: 'Ada Lovelace' } }).toBuffer();

const json = (status, data) =>
  new Response(JSON.stringify(data), { status, headers: { 'Content-Type': 'application/json' } });

// Stands in for GitHub. `overrides` maps "METHOD host/path" to a handler that
// may return or resolve to a Response; returning nothing falls through to the default.
function fakeGitHub(overrides = {}) {
  const calls = [];
  let assets = 0;

  async function fetch(url, init = {}) {
    const call = { url: new URL(url), method: init.method ?? 'GET', headers: new Headers(init.headers), body: init.body };
    calls.push(call);
    const route = `${call.method} ${call.url.host}${call.url.pathname}`;

    const custom = await overrides[route]?.(call);
    if (custom) return custom;

    switch (route) {
      case 'GET api.github.com/repos/saleem-hadad/nodus-feedback':
        return json(200, { id: 123, full_name: 'saleem-hadad/nodus-feedback' });
      case 'POST uploads.github.com/user-attachments/assets':
        assets += 1;
        return json(201, { id: assets, url: `https://github.com/user-attachments/assets/asset-${assets}` });
      case 'POST api.github.com/repos/saleem-hadad/nodus-feedback/issues':
        return json(201, { number: 42, html_url: 'https://github.com/saleem-hadad/nodus-feedback/issues/42' });
      default:
        return json(404, { message: 'Not Found' });
    }
  }

  const to = (method, host, path) =>
    calls.filter((c) => c.method === method && c.url.host === host && (!path || c.url.pathname === path));

  return {
    fetch,
    calls,
    repoLookups: () => to('GET', 'api.github.com', '/repos/saleem-hadad/nodus-feedback'),
    uploads: () => to('POST', 'uploads.github.com'),
    issueCreates: () => to('POST', 'api.github.com', '/repos/saleem-hadad/nodus-feedback/issues'),
  };
}

// A promise and the function that settles it.
function deferred() {
  let resolve;
  const promise = new Promise((r) => (resolve = r));
  return { promise, resolve };
}

function captureLog() {
  const lines = [];
  const push = (...args) => lines.push(args.join(' '));
  return { lines, log: push, info: push, warn: push, error: push };
}

// Starts the app on an ephemeral port, with its own folder for screenshots in
// transit, and stops it when the test ends.
async function start(t, { github = fakeGitHub(), env = ENV, clock = { now: 1_800_000_000_000 } } = {}) {
  const log = captureLog();
  const uploadDir = await mkdtemp(join(tmpdir(), 'nodus-feedback-test-'));
  const app = createApp({ fetch: github.fetch, env, now: () => clock.now, log, uploadDir });
  const server = app.listen(0);
  await once(server, 'listening');
  t.after(async () => {
    await new Promise((resolve) => server.close(resolve));
    await rm(uploadDir, { recursive: true, force: true });
  });
  return { base: `http://127.0.0.1:${server.address().port}`, github, log, clock, uploadDir };
}

// Files are deleted just after the response goes out; give that a moment.
async function leftovers(dir) {
  for (let i = 0; i < 50; i++) {
    const names = await readdir(dir);
    if (names.length === 0) return names;
    await new Promise((r) => setTimeout(r, 10));
  }
  return readdir(dir);
}

function report({ title = 'Board froze', body = 'It froze when I pinched.', screenshots = [], ...rest } = {}) {
  const form = new FormData();
  if (title !== null) form.append('title', title);
  if (body !== null) form.append('body', body);
  for (const [name, value] of Object.entries(rest)) form.append(name, value);
  screenshots.forEach((s, i) => {
    const { bytes, type = 'image/png', name = `shot-${i + 1}.png` } = Buffer.isBuffer(s) ? { bytes: s } : s;
    form.append('screenshots', new Blob([bytes], { type }), name);
  });
  return form;
}

async function send(base, form, { key = KEY, ip } = {}) {
  const headers = {};
  if (key) headers['X-Nodus-Key'] = key;
  if (ip) headers['X-Forwarded-For'] = ip;
  const res = await fetch(`${base}/v1/feedback`, { method: 'POST', headers, body: form });
  const text = await res.text();
  return { status: res.status, headers: res.headers, text, json: JSON.parse(text) };
}

describe('startup', () => {
  test('createApp refuses to start without its required env', () => {
    const args = { fetch: async () => {}, now: Date.now };
    assert.throws(() => createApp({ ...args, env: { NODUS_APP_KEY: KEY } }), /GITHUB_TOKEN/);
    assert.throws(() => createApp({ ...args, env: { GITHUB_TOKEN: TOKEN } }), /NODUS_APP_KEY/);
    assert.throws(() => createApp({ ...args, env: { ...ENV, GITHUB_REPO: 'not a repo' } }), /GITHUB_REPO/);
  });
});

describe('routes', () => {
  test('GET /health answers without a key', async (t) => {
    const { base } = await start(t);
    const res = await fetch(`${base}/health`);
    assert.equal(res.status, 200);
    assert.deepEqual(await res.json(), { ok: true });
  });

  test('unknown routes get a JSON 404', async (t) => {
    const { base } = await start(t);
    const res = await fetch(`${base}/nope`);
    assert.equal(res.status, 404);
    assert.deepEqual(await res.json(), { error: 'Not found.' });
  });
});

describe('app key', () => {
  test('a missing or wrong key gets 401 and never reaches GitHub', async (t) => {
    const { base, github } = await start(t);
    for (const key of [null, 'wrong-key']) {
      const res = await send(base, report(), { key });
      assert.equal(res.status, 401);
      assert.equal(typeof res.json.error, 'string');
    }
    assert.equal(github.calls.length, 0);
  });
});

describe('validation', () => {
  const cases = [
    ['missing title', () => report({ title: null }), 400, 'Add a title.'],
    ['blank title', () => report({ title: '   \n ' }), 400, 'Add a title.'],
    ['long title', () => report({ title: 'x'.repeat(257) }), 400, /title is too long/],
    ['missing body', () => report({ body: null }), 400, 'Add a description.'],
    ['long body', () => report({ body: 'x'.repeat(20_001) }), 400, /description is too long/],
    ['bad reporter', () => report({ reporter: 'not a user!' }), 400, "That isn't a GitHub username."],
    ['reporter with a leading hyphen', () => report({ reporter: '-octocat' }), 400, "That isn't a GitHub username."],
    ['six screenshots', () => report({ screenshots: Array(6).fill(PNG) }), 400, /at most 5 screenshots/],
    [
      'an 11 MB screenshot',
      () => report({ screenshots: [Buffer.concat([PNG, Buffer.alloc(11 * 1024 * 1024)])] }),
      413,
      /10 MB/,
    ],
    [
      'a text file pretending to be a PNG',
      () => report({ screenshots: [{ bytes: Buffer.from('hello, not an image'), type: 'image/png', name: 'shot.png' }] }),
      400,
      "Screenshot 1 isn't a PNG, JPEG, GIF or WebP image.",
    ],
    [
      'a PNG header with junk behind it',
      () => report({ screenshots: [Buffer.concat([PNG.subarray(0, 8), Buffer.alloc(1000, 7)])] }),
      400,
      "Screenshot 1 couldn't be read as an image.",
    ],
    [
      'a file in another field',
      () => {
        const form = report();
        form.append('attachment', new Blob([PNG], { type: 'image/png' }), 'a.png');
        return form;
      },
      400,
      'Only screenshots can be attached.',
    ],
    [
      'a repeated title field',
      () => {
        const form = report();
        form.append('title', 'again');
        return form;
      },
      400,
      'Send title once, as plain text.',
    ],
  ];

  for (const [name, makeForm, status, message] of cases) {
    test(`${name} → ${status}`, async (t) => {
      const { base, github } = await start(t);
      const res = await send(base, makeForm());
      assert.equal(res.status, status);
      if (message instanceof RegExp) assert.match(res.json.error, message);
      else assert.equal(res.json.error, message);
      assert.equal(github.calls.length, 0, 'nothing reaches GitHub');
    });
  }

  test('a non-multipart request is turned away', async (t) => {
    const { base } = await start(t);
    const res = await fetch(`${base}/v1/feedback`, {
      method: 'POST',
      headers: { 'X-Nodus-Key': KEY, 'Content-Type': 'application/json' },
      body: JSON.stringify({ title: 'x', body: 'y' }),
    });
    assert.equal(res.status, 400);
    assert.deepEqual(await res.json(), { error: 'Send the report as multipart/form-data.' });
  });

  test('limits count characters: 20,000 emoji is a valid description', async (t) => {
    const { base, github } = await start(t);
    const res = await send(base, report({ body: '😀'.repeat(20_000) }));
    assert.equal(res.status, 201);
    assert.equal(github.issueCreates().length, 1);
  });

  test('newlines count once even though FormData sends them as CRLF', async (t) => {
    const { base, github } = await start(t);
    const res = await send(base, report({ body: 'line\n'.repeat(4_000) })); // 19,999 characters once trimmed
    assert.equal(res.status, 201);
    assert.ok(!JSON.parse(github.issueCreates()[0].body).body.includes('\r'));
  });

  test('reports with bad text do not use up the rate limit, screenshots or not', async (t) => {
    const { base } = await start(t);
    for (let i = 0; i < 8; i++) {
      const res = await send(base, report({ title: null, screenshots: i % 2 ? [PNG] : [] }), { ip: '198.51.100.1' });
      assert.equal(res.status, 400);
      assert.equal(res.json.error, 'Add a title.');
    }
    assert.equal((await send(base, report(), { ip: '198.51.100.1' })).status, 201);
  });

  test('a report counts once its screenshots start, even if one is then turned down', async (t) => {
    const { base } = await start(t);
    const junk = { bytes: Buffer.from('not an image'), type: 'image/png', name: 'a.png' };
    for (let i = 0; i < 5; i++) {
      assert.equal((await send(base, report({ screenshots: [junk] }), { ip: '198.51.100.2' })).status, 400);
    }
    assert.equal((await send(base, report(), { ip: '198.51.100.2' })).status, 429);
  });
});

describe('happy path', () => {
  test('uploads screenshots, creates the issue and answers 201', async (t) => {
    const { base, github } = await start(t);
    const form = report({
      title: 'Board  froze\n when @bob pinches',
      body: 'Reported by @OctoCat\nThe board froze after I pinched. cc @someone, or mail me@example.com',
      reporter: 'octocat',
      version: '1.0.2',
      os: 'macOS 27.2',
      arch: 'arm64',
      screenshots: [PNG, { bytes: JPEG, type: 'image/jpeg', name: 'IMG_0001.JPG' }],
    });

    const res = await send(base, form);
    assert.equal(res.status, 201);
    assert.deepEqual(res.json, { number: 42, url: 'https://github.com/saleem-hadad/nodus-feedback/issues/42' });

    // Every call carries the same auth and API headers.
    for (const call of github.calls) {
      assert.equal(call.headers.get('authorization'), `Bearer ${TOKEN}`);
      assert.equal(call.headers.get('accept'), 'application/vnd.github+json');
      assert.equal(call.headers.get('x-github-api-version'), '2022-11-28');
      assert.equal(call.headers.get('user-agent'), 'nodus-feedback-server');
    }

    // Order: repository id, then each upload, then the issue.
    assert.deepEqual(
      github.calls.map((c) => `${c.method} ${c.url.host}${c.url.pathname}`),
      [
        'GET api.github.com/repos/saleem-hadad/nodus-feedback',
        'POST uploads.github.com/user-attachments/assets',
        'POST uploads.github.com/user-attachments/assets',
        'POST api.github.com/repos/saleem-hadad/nodus-feedback/issues',
      ],
    );

    const [first, second] = github.uploads();
    const expected = [
      [first, PNG, 'screenshot-1.png', 'image/png'],
      [second, JPEG, 'screenshot-2.jpg', 'image/jpeg'],
    ];
    for (const [call, , name, type] of expected) {
      assert.equal(call.url.searchParams.get('name'), name);
      assert.equal(call.url.searchParams.get('content_type'), type);
      assert.equal(call.url.searchParams.get('repository_id'), '123');
      assert.equal(call.headers.get('content-type'), 'application/octet-stream');
      assert.equal(call.headers.get('content-length'), String(call.body.length));
      const uploaded = await sharp(call.body).metadata();
      assert.deepEqual([uploaded.format, uploaded.width, uploaded.height], [type.slice(6), 40, 30]);
    }

    const [create] = github.issueCreates();
    assert.equal(create.headers.get('content-type'), 'application/json');
    assert.deepEqual(JSON.parse(create.body), {
      title: `Board froze when @${WJ}bob pinches`,
      body:
        '<sub>Sent from Nodus feedback · 1.0.2 · macOS 27.2 · arm64</sub>\n\n' +
        'Reported by @OctoCat\n' +
        `The board froze after I pinched. cc @${WJ}someone, or mail me@example.com\n\n` +
        '![Screenshot 1](https://github.com/user-attachments/assets/asset-1)\n\n' +
        '![Screenshot 2](https://github.com/user-attachments/assets/asset-2)',
      labels: ['feedback'],
    });
  });

  test('screenshots are re-encoded: no metadata, nothing riding behind the image', async (t) => {
    const { base, github } = await start(t);
    const payload = Buffer.from('PK\x03\x04 a zip hiding behind the PNG');
    const res = await send(base, report({ screenshots: [Buffer.concat([PNG, payload]), JPEG] }));
    assert.equal(res.status, 201);

    const [png, jpeg] = github.uploads().map((c) => Buffer.from(c.body));
    assert.ok(!png.includes(payload), 'the payload is gone');
    assert.ok((await sharp(JPEG).metadata()).exif, 'the original JPEG has EXIF');
    assert.equal((await sharp(jpeg).metadata()).exif, undefined);
  });

  test('the repository id is looked up once and reused', async (t) => {
    const { base, github } = await start(t);
    assert.equal((await send(base, report({ screenshots: [PNG] }))).status, 201);
    assert.equal((await send(base, report({ screenshots: [JPEG] }))).status, 201);
    assert.equal(github.repoLookups().length, 1);
    assert.equal(github.uploads().length, 2);
  });

  test('a report without screenshots skips the lookup and uploads', async (t) => {
    const { base, github } = await start(t);
    assert.equal((await send(base, report())).status, 201);
    assert.equal(github.calls.length, 1);
    assert.equal(github.issueCreates().length, 1);
  });

  test('GITHUB_REPO and ISSUE_LABEL are honoured', async (t) => {
    const github = fakeGitHub({
      'POST api.github.com/repos/acme/widgets/issues': () =>
        json(201, { number: 7, html_url: 'https://github.com/acme/widgets/issues/7' }),
    });
    const { base } = await start(t, { github, env: { ...ENV, GITHUB_REPO: 'acme/widgets', ISSUE_LABEL: 'bug' } });
    const res = await send(base, report());
    assert.deepEqual(res.json, { number: 7, url: 'https://github.com/acme/widgets/issues/7' });
    assert.deepEqual(JSON.parse(github.calls[0].body).labels, ['bug']);
  });
});

describe('GitHub failures', () => {
  test('a failed upload fails the report with 502 and creates no issue', async (t) => {
    const github = fakeGitHub({
      'POST uploads.github.com/user-attachments/assets': () => json(404, { message: 'Not Found' }),
    });
    const { base, log } = await start(t, { github });
    const res = await send(base, report({ screenshots: [PNG, JPEG] }));
    assert.equal(res.status, 502);
    assert.deepEqual(res.json, { error: "GitHub didn't accept the report. Try again in a minute." });
    assert.equal(github.uploads().length, 1, 'stops at the first failure');
    assert.equal(github.issueCreates().length, 0);
    assert.ok(log.lines.some((l) => l.includes('Screenshot upload failed: 404 Not Found')));
  });

  test('a missing label (422) is retried once without labels', async (t) => {
    const github = fakeGitHub({
      'POST api.github.com/repos/saleem-hadad/nodus-feedback/issues': (call) => {
        if (JSON.parse(call.body).labels) {
          return json(422, { message: 'Validation Failed', errors: [{ resource: 'Label', code: 'invalid' }] });
        }
      },
    });
    const { base, log } = await start(t, { github });
    const res = await send(base, report());
    assert.equal(res.status, 201);
    assert.deepEqual(res.json, { number: 42, url: 'https://github.com/saleem-hadad/nodus-feedback/issues/42' });

    const creates = github.issueCreates().map((c) => JSON.parse(c.body));
    assert.equal(creates.length, 2);
    assert.deepEqual(creates[0].labels, ['feedback']);
    assert.equal('labels' in creates[1], false);
    assert.ok(log.lines.some((l) => l.includes('retrying without the "feedback" label')));
  });

  test("a second 422 is a normal failure, not a loop", async (t) => {
    const github = fakeGitHub({
      'POST api.github.com/repos/saleem-hadad/nodus-feedback/issues': () => json(422, { message: 'Validation Failed' }),
    });
    const { base } = await start(t, { github });
    assert.equal((await send(base, report())).status, 502);
    assert.equal(github.issueCreates().length, 2);
  });

  test("GitHub's response and the token never reach the client or the logs", async (t) => {
    const leaky = `Bad credentials for ${TOKEN} SECRET-DETAIL`;
    const github = fakeGitHub({
      'POST api.github.com/repos/saleem-hadad/nodus-feedback/issues': () =>
        json(401, { message: 'Bad credentials', documentation_url: 'https://docs.github.com', leaky }),
    });
    const { base, log } = await start(t, { github });
    const res = await send(base, report());
    assert.equal(res.status, 502);
    assert.deepEqual(res.json, { error: "GitHub didn't accept the report. Try again in a minute." });
    for (const secret of [TOKEN, 'SECRET-DETAIL', 'Bad credentials', 'docs.github.com']) {
      assert.ok(!res.text.includes(secret), `response leaks ${secret}`);
    }
    assert.ok(log.lines.some((l) => l.includes('Issue create failed: 401 Bad credentials')));
    assert.ok(!log.lines.some((l) => l.includes(TOKEN) || l.includes('SECRET-DETAIL')));
  });

  test('a network failure is a 502', async (t) => {
    const github = fakeGitHub({
      'POST api.github.com/repos/saleem-hadad/nodus-feedback/issues': () => {
        throw new TypeError('fetch failed', { cause: new Error('getaddrinfo ENOTFOUND api.github.com') });
      },
    });
    const { base, log } = await start(t, { github });
    assert.equal((await send(base, report())).status, 502);
    assert.ok(log.lines.some((l) => l.includes('ENOTFOUND')));
  });

  test('a failed repository lookup is not cached', async (t) => {
    let failNext = true;
    const github = fakeGitHub({
      'GET api.github.com/repos/saleem-hadad/nodus-feedback': () => {
        if (failNext) {
          failNext = false;
          return json(503, { message: 'Service Unavailable' });
        }
      },
    });
    const { base } = await start(t, { github });
    assert.equal((await send(base, report({ screenshots: [PNG] }))).status, 502);
    assert.equal((await send(base, report({ screenshots: [PNG] }))).status, 201);
    assert.equal(github.repoLookups().length, 2);
  });
});

describe('screenshots in transit', () => {
  test('are deleted whether the report is posted or turned down', async (t) => {
    const github = fakeGitHub();
    const { base, uploadDir } = await start(t, { github });
    const junk = { bytes: Buffer.from('not an image'), type: 'image/png', name: 'a.png' };
    const huge = Buffer.concat([PNG, Buffer.alloc(11 * 1024 * 1024)]);
    const cases = [
      [report({ screenshots: [PNG, JPEG] }), 201],
      [report({ screenshots: [PNG, junk] }), 400],
      [report({ screenshots: [PNG, huge] }), 413],
    ];
    for (const [i, [form, status]] of cases.entries()) {
      assert.equal((await send(base, form, { ip: `198.51.100.${30 + i}` })).status, status);
      assert.deepEqual(await leftovers(uploadDir), [], `after the ${status}`);
    }
  });

  test('are deleted when GitHub fails', async (t) => {
    const github = fakeGitHub({ 'POST uploads.github.com/user-attachments/assets': () => json(500, {}) });
    const { base, uploadDir } = await start(t, { github });
    assert.equal((await send(base, report({ screenshots: [PNG, JPEG] }))).status, 502);
    assert.deepEqual(await leftovers(uploadDir), []);
  });

  test('a client that leaves mid-upload frees its place and its files', async (t) => {
    const { base, uploadDir } = await start(t);
    const boundary = 'nodus-test-boundary';
    const head =
      `--${boundary}\r\nContent-Disposition: form-data; name="title"\r\n\r\nIt broke\r\n` +
      `--${boundary}\r\nContent-Disposition: form-data; name="body"\r\n\r\nDetails\r\n` +
      `--${boundary}\r\nContent-Disposition: form-data; name="screenshots"; filename="a.png"\r\n` +
      'Content-Type: image/png\r\n\r\n';

    // Two uploads that stall after their first bytes, then go away.
    for (let i = 0; i < 2; i++) {
      const leave = new AbortController();
      const body = new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode(head));
          controller.enqueue(PNG.subarray(0, 100));
        },
      });
      const sent = fetch(`${base}/v1/feedback`, {
        method: 'POST',
        headers: { 'X-Nodus-Key': KEY, 'Content-Type': `multipart/form-data; boundary=${boundary}` },
        body,
        duplex: 'half',
        signal: leave.signal,
      }).catch(() => {});
      while ((await readdir(uploadDir)).length === 0) await new Promise((r) => setTimeout(r, 5));
      leave.abort();
      await sent;
      assert.deepEqual(await leftovers(uploadDir), []);
    }

    assert.equal((await send(base, report({ screenshots: [PNG] }))).status, 201);
  });
});

describe('rate limits', () => {
  test('five reports per IP per rolling hour', async (t) => {
    const { base, clock } = await start(t);
    const ip = '203.0.113.7';

    for (let i = 0; i < 5; i++) assert.equal((await send(base, report(), { ip })).status, 201);

    const limited = await send(base, report(), { ip });
    assert.equal(limited.status, 429);
    assert.equal(limited.headers.get('retry-after'), '3600');
    assert.match(limited.json.error, /Try again in 60 minutes\./);

    // Someone else is unaffected.
    assert.equal((await send(base, report(), { ip: '203.0.113.8' })).status, 201);

    // Half an hour later the window still holds all five.
    clock.now += HOUR / 2;
    const stillLimited = await send(base, report(), { ip });
    assert.equal(stillLimited.status, 429);
    assert.equal(stillLimited.headers.get('retry-after'), '1800');

    // Once the first five leave the window, the IP may send again.
    clock.now += HOUR / 2;
    assert.equal((await send(base, report(), { ip })).status, 201);
  });

  test('sixty reports per rolling hour across everyone', async (t) => {
    const { base, github, clock } = await start(t);
    for (let i = 0; i < 60; i++) {
      assert.equal((await send(base, report(), { ip: `192.0.2.${Math.floor(i / 5)}` })).status, 201);
    }

    const limited = await send(base, report(), { ip: '192.0.2.200' });
    assert.equal(limited.status, 429);
    assert.equal(limited.headers.get('retry-after'), '3600');
    assert.match(limited.json.error, /getting a lot of reports/);
    assert.equal(github.issueCreates().length, 60);

    clock.now += HOUR;
    assert.equal((await send(base, report(), { ip: '192.0.2.200' })).status, 201);
  });

  test('two reports with screenshots at a time; the rest wait, text-only ones do not', async (t) => {
    const gate = deferred();
    const github = fakeGitHub({ 'POST uploads.github.com/user-attachments/assets': () => gate.promise.then(() => undefined) });
    const { base } = await start(t, { github });

    const busy = [1, 2].map((n) => send(base, report({ screenshots: [PNG] }), { ip: `203.0.113.${20 + n}` }));
    while (github.uploads().length < 2) await new Promise((r) => setTimeout(r, 5));

    const third = await send(base, report({ screenshots: [PNG] }), { ip: '203.0.113.23' });
    assert.equal(third.status, 429);
    assert.equal(third.headers.get('retry-after'), '60');
    assert.equal(third.json.error, 'Nodus is sending other reports right now. Try again in a minute.');
    assert.equal((await send(base, report(), { ip: '203.0.113.24' })).status, 201);

    gate.resolve();
    assert.deepEqual((await Promise.all(busy)).map((r) => r.status), [201, 201]);
    assert.equal((await send(base, report({ screenshots: [PNG] }), { ip: '203.0.113.23' })).status, 201);
  });

  test('a limited client is turned away before its upload is read', async (t) => {
    const { base, github } = await start(t);
    const ip = '203.0.113.9';
    for (let i = 0; i < 5; i++) await send(base, report(), { ip });
    const callsBefore = github.calls.length;
    const res = await send(base, report({ screenshots: [PNG] }), { ip });
    assert.equal(res.status, 429);
    assert.equal(github.calls.length, callsBefore);
  });
});
