// The HTTP side: checks the app key, reads and validates the report, applies
// the rate limits, re-encodes the screenshots, then hands off to GitHub. Fetch,
// env and the clock are injected so tests can run the whole thing without
// GitHub or real time.

import { open, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import express from 'express';
import multer from 'multer';
import { createGitHub, GitHubError } from './github.js';
import { ImageError, reencode } from './images.js';
import { createRateLimiter } from './rate-limit.js';
import {
  BODY_MAX,
  TITLE_MAX,
  buildBody,
  buildTitle,
  charCount,
  cleanMeta,
  isValidUsername,
} from './issue.js';

const MAX_SCREENSHOTS = 5;
const MAX_SCREENSHOT_BYTES = 10 * 1024 * 1024; // GitHub's limit for an image
const REPORTS_PER_CLIENT = 5; // per IP, per rolling hour
const REPORTS_OVERALL = 60; // across everyone, per rolling hour
// Reports whose screenshots are being received, re-encoded or uploaded at
// once. Each can bring 50 MB of them, so this bounds the work and the disk.
const SCREENSHOT_REPORTS_AT_ONCE = 2;

const REPO_PATTERN = /^[A-Za-z0-9-]+\/[A-Za-z0-9._-]+$/;

// An error whose message is safe to show the user as-is.
class HttpError extends Error {
  constructor(status, message, headers = {}) {
    super(message);
    this.status = status;
    this.headers = headers;
  }
}

const badRequest = (message) => new HttpError(400, message);

export function createApp({ fetch, env, now, log = console, uploadDir = tmpdir() }) {
  const token = env.GITHUB_TOKEN;
  const appKey = env.NODUS_APP_KEY;
  const repo = env.GITHUB_REPO || 'saleem-hadad/nodus-feedback';
  const label = env.ISSUE_LABEL || 'feedback';

  const missing = [!token && 'GITHUB_TOKEN', !appKey && 'NODUS_APP_KEY'].filter(Boolean);
  if (missing.length > 0) {
    throw new Error(`Missing required environment variable(s): ${missing.join(', ')}.`);
  }
  if (!REPO_PATTERN.test(repo)) {
    throw new Error(`GITHUB_REPO must look like "owner/name", got "${repo}".`);
  }

  const github = createGitHub({ fetch, token, repo, log });
  const limiter = createRateLimiter({ now, perClient: REPORTS_PER_CLIENT, overall: REPORTS_OVERALL });

  // Reports with screenshots are counted when their first screenshot starts
  // to arrive, before any of its bytes are kept. The app sends the text
  // first, so a report that would be turned away anyway is turned away here,
  // and nobody can make us hold 50 MB of junk over and over for free.
  const admitted = new WeakSet();
  let reportsWithScreenshots = 0;

  function admitScreenshots(req) {
    if (admitted.has(req)) return;
    readReport(req.body);
    throwIfLimited(limiter.check(clientOf(req)));
    if (reportsWithScreenshots >= SCREENSHOT_REPORTS_AT_ONCE) {
      throw new HttpError(429, 'Nodus is sending other reports right now. Try again in a minute.', {
        'Retry-After': '60',
      });
    }
    limiter.record(clientOf(req));
    admitted.add(req);
    reportsWithScreenshots += 1;
  }

  const parseForm = multer({
    // Screenshots stream to disk rather than memory, which a small instance
    // has little of; they're deleted when the request ends (see removeFiles).
    storage: multer.diskStorage({ destination: uploadDir }),
    limits: {
      fileSize: MAX_SCREENSHOT_BYTES,
      files: MAX_SCREENSHOTS,
      fields: 10,
      // 20,000 characters can take up to 80 KB in UTF-8, so a valid
      // description always fits; anything bigger is junk.
      fieldSize: 100 * 1024,
    },
    fileFilter(req, file, accept) {
      try {
        admitScreenshots(req);
        accept(null, true);
      } catch (err) {
        accept(err);
      }
    },
  }).array('screenshots', MAX_SCREENSHOTS);

  const app = express();
  // One proxy hop sits in front of the app; req.ip is the address it saw.
  app.set('trust proxy', 1);
  app.disable('x-powered-by');

  app.get('/health', (req, res) => {
    res.json({ ok: true });
  });

  app.post('/v1/feedback', async (req, res) => {
    // The key ships inside every app build, so it isn't a secret and a plain
    // comparison is fine: it only turns away generic bots. It's checked before
    // the body is read so they can't make us buffer uploads.
    if (req.get('X-Nodus-Key') !== appKey) {
      throw new HttpError(401, "This app key isn't valid.");
    }

    // Someone already over the limit gets turned away before we read anything.
    const client = clientOf(req);
    throwIfLimited(limiter.check(client));

    if (!req.is('multipart/form-data')) {
      throw badRequest('Send the report as multipart/form-data.');
    }

    try {
      await readForm(parseForm, req, res);
      const report = readReport(req.body);

      // A report with screenshots was counted as they arrived; count one
      // without them now. Check again, since other reports may have been
      // counted while this one was read.
      if (!admitted.has(req)) {
        throwIfLimited(limiter.check(client));
        limiter.record(client);
      }
      const screenshots = await prepareScreenshots(req.files ?? []);

      // One at a time, so the first failure stops the rest; an upload failure
      // fails the whole report rather than leaving a half-written issue.
      const screenshotUrls = [];
      for (const { path, ...screenshot } of screenshots) {
        screenshotUrls.push(await github.uploadScreenshot({ ...screenshot, bytes: await readFile(path) }));
      }

      const issue = await github.createIssue({
        title: buildTitle(report.title),
        body: buildBody({ ...report, screenshotUrls }),
        label,
      });
      log.info(`Created issue #${issue.number} with ${screenshotUrls.length} screenshot(s).`);
      res.status(201).json(issue);
    } finally {
      // Posted or turned down, the report is done with its screenshots. multer
      // gives up on a client that goes away mid-upload, so this always runs.
      if (admitted.has(req)) reportsWithScreenshots -= 1;
      removeFiles(req.files, log);
    }
  });

  app.use((req, res) => {
    res.status(404).json({ error: 'Not found.' });
  });

  // Every error leaves as {"error": "<one human sentence>"}. GitHub's own
  // response, and anything that could carry the token, stays in the logs.
  app.use((err, req, res, next) => {
    if (res.headersSent) return next(err);

    if (err instanceof HttpError) {
      return res.status(err.status).set(err.headers).json({ error: err.message });
    }
    if (err instanceof GitHubError) {
      log.error(`GitHub: ${err.message}`);
      return res.status(502).json({ error: "GitHub didn't accept the report. Try again in a minute." });
    }

    // Express's own client errors, such as a malformed URL.
    const status = err.status ?? err.statusCode;
    if (status >= 400 && status < 500) {
      return res.status(status).json({ error: "That request couldn't be read." });
    }

    log.error(err.stack ?? String(err));
    res.status(500).json({ error: 'Something went wrong on our side. Try again in a minute.' });
  });

  return app;
}

function throwIfLimited(limit) {
  if (!limit) return;
  const minutes = Math.ceil(limit.retryAfter / 60);
  const wait = `Try again in ${minutes} minute${minutes === 1 ? '' : 's'}.`;
  const message =
    limit.scope === 'client'
      ? `You've sent a lot of reports in the last hour. ${wait}`
      : `Nodus is getting a lot of reports right now. ${wait}`;
  throw new HttpError(429, message, { 'Retry-After': String(limit.retryAfter) });
}

// multer speaks callbacks; turn its errors into messages a person can act on.
function readForm(parse, req, res) {
  return new Promise((resolve, reject) => {
    parse(req, res, (err) => (err ? reject(formError(err)) : resolve()));
  });
}

function formError(err) {
  if (err instanceof HttpError) return err; // from admitScreenshots
  if (!(err instanceof multer.MulterError)) {
    // busboy couldn't parse the body: truncated upload, bad boundary, etc.
    return badRequest("That report couldn't be read. Try sending it again.");
  }
  switch (err.code) {
    case 'LIMIT_FILE_SIZE':
      return new HttpError(413, 'Each screenshot must be 10 MB or smaller.');
    case 'LIMIT_FILE_COUNT':
      return badRequest(`Attach at most ${MAX_SCREENSHOTS} screenshots.`);
    case 'LIMIT_UNEXPECTED_FILE':
      // .array() reports going over its count this way too.
      return err.field === 'screenshots'
        ? badRequest(`Attach at most ${MAX_SCREENSHOTS} screenshots.`)
        : badRequest('Only screenshots can be attached.');
    case 'LIMIT_FIELD_VALUE':
      if (err.field === 'title') return new HttpError(413, 'The title is too long.');
      if (err.field === 'body') return new HttpError(413, 'The description is too long.');
      return new HttpError(413, 'One of the report fields is too long.');
    default:
      return badRequest("That report has fields we don't expect.");
  }
}

// The report's text. It's read again once the whole form is in, since a
// report with screenshots was already checked as its first one arrived.
function readReport(form) {
  const title = readText(form, 'title');
  if (!title) throw badRequest('Add a title.');
  if (charCount(title) > TITLE_MAX) {
    throw badRequest(`The title is too long (${TITLE_MAX} characters at most).`);
  }

  const body = readText(form, 'body');
  if (!body) throw badRequest('Add a description.');
  if (charCount(body) > BODY_MAX) {
    throw badRequest(`The description is too long (${BODY_MAX.toLocaleString('en-US')} characters at most).`);
  }

  const reporter = readText(form, 'reporter');
  if (reporter && !isValidUsername(reporter)) throw badRequest("That isn't a GitHub username.");

  return {
    title,
    body,
    reporter,
    version: cleanMeta(form.version),
    os: cleanMeta(form.os),
    arch: cleanMeta(form.arch),
  };
}

// Checks every screenshot's type before decoding any, then re-encodes each
// into a file beside it.
async function prepareScreenshots(files) {
  const types = [];
  for (const [i, file] of files.entries()) {
    const type = sniffImage(await readHead(file.path));
    if (!type) throw badRequest(`Screenshot ${i + 1} isn't a PNG, JPEG, GIF or WebP image.`);
    types.push(type);
  }

  const screenshots = [];
  for (const [i, file] of files.entries()) {
    let image;
    try {
      image = await reencode(file.path, reencodedPath(file), types[i].ext, MAX_SCREENSHOT_BYTES);
    } catch (err) {
      if (err instanceof ImageError) throw new HttpError(err.status, `Screenshot ${i + 1} ${err.message}`);
      throw err;
    }
    screenshots.push({ name: `screenshot-${i + 1}.${image.ext}`, contentType: image.mime, path: reencodedPath(file) });
  }
  return screenshots;
}

// Enough of a file to tell its type.
async function readHead(path) {
  const file = await open(path);
  try {
    const { buffer, bytesRead } = await file.read(Buffer.alloc(12), 0, 12, 0);
    return buffer.subarray(0, bytesRead);
  } finally {
    await file.close();
  }
}

const reencodedPath = (file) => `${file.path}.reencoded`;

// multer deletes what it saved when it fails partway; everything else goes here.
function removeFiles(files = [], log) {
  for (const file of files.filter((f) => f.path)) {
    for (const path of [file.path, reencodedPath(file)]) {
      rm(path, { force: true }).catch((err) => log.error(`Couldn't delete ${path}: ${err.message}`));
    }
  }
}

// Who the rate limits count against.
function clientOf(req) {
  return req.ip ?? 'unknown';
}

// multer turns a repeated or bracketed field name into an array or object;
// only a single plain string is a valid answer. Form encoders (browsers,
// undici) send newlines as CRLF; store and count them as "\n" like the app.
function readText(form, name) {
  const value = form[name];
  if (value === undefined) return '';
  if (typeof value !== 'string') throw badRequest(`Send ${name} once, as plain text.`);
  return value.replace(/\r\n?/g, '\n').trim();
}

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

// The type comes from the file's first bytes. The declared mimetype and file
// name are whatever the client says, so they decide nothing.
export function sniffImage(bytes) {
  const ascii = (start, end) => bytes.subarray(start, end).toString('latin1');

  if (bytes.subarray(0, 8).equals(PNG_SIGNATURE)) return { mime: 'image/png', ext: 'png' };
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return { mime: 'image/jpeg', ext: 'jpg' };
  if (ascii(0, 6) === 'GIF87a' || ascii(0, 6) === 'GIF89a') return { mime: 'image/gif', ext: 'gif' };
  if (ascii(0, 4) === 'RIFF' && ascii(8, 12) === 'WEBP') return { mime: 'image/webp', ext: 'webp' };
  return null;
}
