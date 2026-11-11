// Text rules for turning a report into a GitHub issue. Pure functions: no I/O.

export const TITLE_MAX = 256;
export const BODY_MAX = 20_000;
const META_MAX = 40;

// GitHub's own username rule: letters, digits and single hyphens, no hyphen at
// either end, 39 characters at most.
const USERNAME = /^[A-Za-z0-9](?:[A-Za-z0-9]|-(?=[A-Za-z0-9])){0,38}$/;

// An "@" starts a mention when no word character sits right before it (so
// "a@b.com" stays an email) and a name character follows it. This mirrors the
// rule GitHub uses to find mentions, but neutralizes even the odd cases GitHub
// would skip: when in doubt, nobody gets pinged.
const MENTION = /(?<![A-Za-z0-9_])@(?=[A-Za-z0-9])/g;

// U+2060 WORD JOINER is invisible, but sitting between "@" and the name it
// stops GitHub from seeing a mention, so the server can't be used to ping people.
const WORD_JOINER = '⁠';

const REPORTER_PREFIX = 'Reported by @';

// Lengths count characters the way the Rust app does (Unicode scalar values),
// not UTF-16 code units, so an emoji counts once on both sides.
export function charCount(text) {
  return [...text].length;
}

export function isValidUsername(name) {
  return typeof name === 'string' && USERNAME.test(name);
}

export function neutralizeMentions(text) {
  return text.replace(MENTION, `@${WORD_JOINER}`);
}

// version, os and arch only ever land in the footer. Keeping them to a small
// alphabet means they can't carry markdown, HTML or mentions into the issue.
export function cleanMeta(value) {
  if (typeof value !== 'string') return '';
  return value
    .replace(/[^A-Za-z0-9 ._()-]/g, '')
    .replace(/ +/g, ' ')
    .trim()
    .slice(0, META_MAX)
    .trim();
}

export function buildTitle(title) {
  const oneLine = neutralizeMentions(title.replace(/\s+/g, ' ').trim());
  // Word joiners add characters; stay inside GitHub's title limit. Cutting here
  // can never re-arm a mention: at worst it drops the name after an "@".
  return [...oneLine].slice(0, TITLE_MAX).join('');
}

export function buildBody({ body, reporter, screenshotUrls = [], version, os, arch }) {
  const parts = [neutralizeBody(body, reporter)];

  if (screenshotUrls.length > 0) {
    parts.push(screenshotUrls.map((url, i) => `![Screenshot ${i + 1}](${url})`).join('\n\n'));
  }

  const footer = ['Sent from Nodus feedback', version, os, arch].filter(Boolean).join(' · ');
  parts.push(`---\n<sub>${footer}</sub>`);

  return parts.join('\n\n');
}

// Neutralizes every mention, except a first line that reads exactly
// "Reported by @<reporter>": that one should notify the reporter, so they can
// follow the issue they filed.
function neutralizeBody(body, reporter) {
  if (!isValidUsername(reporter)) return neutralizeMentions(body);

  const end = body.indexOf('\n');
  const firstLine = end === -1 ? body : body.slice(0, end);
  const rest = end === -1 ? '' : body.slice(end);

  const line = firstLine.trimEnd();
  const isReporterLine =
    line.startsWith(REPORTER_PREFIX) &&
    line.slice(REPORTER_PREFIX.length).toLowerCase() === reporter.toLowerCase();

  if (!isReporterLine) return neutralizeMentions(body);
  return firstLine + neutralizeMentions(rest);
}
