// The GitHub calls a report needs. Everything goes through the injected fetch,
// so tests can stand in for GitHub without a network or a token.

const API = 'https://api.github.com';
const UPLOADS = 'https://uploads.github.com';

// Long enough for a 10 MB upload on a slow day, short enough that a stuck call
// fails the request instead of hanging it.
const TIMEOUT_MS = 30_000;

// Carries only GitHub's status and its short `message`. It is logged on the
// server and never sent to the client.
export class GitHubError extends Error {
  constructor(step, status, message) {
    super(`${step} failed: ${status || 'no response'} ${message}`.trim());
    this.name = 'GitHubError';
    this.step = step;
    this.status = status;
  }
}

export function createGitHub({ fetch, token, repo, log = console }) {
  const baseHeaders = {
    Authorization: `Bearer ${token}`,
    Accept: 'application/vnd.github+json',
    'X-GitHub-Api-Version': '2022-11-28',
    'User-Agent': 'nodus-feedback-server',
  };

  async function request(step, url, { headers, ...init } = {}) {
    let res;
    try {
      res = await fetch(url, {
        ...init,
        headers: { ...baseHeaders, ...headers },
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
    } catch (err) {
      // Network failure or timeout; fetch keeps the useful detail in `cause`.
      throw new GitHubError(step, 0, err.cause?.message ?? err.message);
    }

    const data = await res.json().catch(() => null);
    if (!res.ok) throw new GitHubError(step, res.status, data?.message ?? res.statusText);
    return data;
  }

  // The repository id never changes, so look it up once per process. Caching
  // the promise means concurrent first requests share one lookup; a failure is
  // forgotten so the next report tries again.
  let repoId = null;
  function repositoryId() {
    repoId ??= request('Repository lookup', `${API}/repos/${repo}`)
      .then((data) => {
        if (!Number.isInteger(data?.id)) {
          throw new GitHubError('Repository lookup', 200, 'response had no id');
        }
        return data.id;
      })
      .catch((err) => {
        repoId = null;
        throw err;
      });
    return repoId;
  }

  // Same endpoint `gh issue create --attach` uses (cli/cli
  // internal/attachments/client.go). It answers 404 when the token can't write
  // to the repository. Uploads can't be undone, so only call this once the
  // whole report has passed validation.
  async function uploadScreenshot({ name, contentType, bytes }) {
    const id = await repositoryId();
    const query = new URLSearchParams({ name, content_type: contentType, repository_id: String(id) });
    const data = await request('Screenshot upload', `${UPLOADS}/user-attachments/assets?${query}`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/octet-stream',
        'Content-Length': String(bytes.length),
      },
      body: bytes,
    });
    if (typeof data?.url !== 'string') {
      throw new GitHubError('Screenshot upload', 200, 'response had no url');
    }
    return data.url;
  }

  async function createIssue({ title, body, label }) {
    const create = (payload) =>
      request('Issue create', `${API}/repos/${repo}/issues`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });

    let data;
    try {
      data = await create({ title, body, labels: [label] });
    } catch (err) {
      // A missing label shouldn't cost the user their report. Any other 422
      // fails again below and surfaces as a normal GitHub error.
      if (!(err instanceof GitHubError && err.status === 422)) throw err;
      log.warn(`${err.message}; retrying without the "${label}" label. Does the label exist?`);
      data = await create({ title, body });
    }

    if (!Number.isInteger(data?.number) || typeof data?.html_url !== 'string') {
      throw new GitHubError('Issue create', 200, 'response had no number or html_url');
    }
    return { number: data.number, url: data.html_url };
  }

  return { repositoryId, uploadScreenshot, createIssue };
}
