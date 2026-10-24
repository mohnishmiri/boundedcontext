import { http, HttpResponse } from 'msw';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { apiError, server } from '../test/server';
import {
  ApiError,
  cancelJob,
  createJob,
  getJob,
  getMe,
  listRepositories,
  parseRetryAfter,
  setUnauthorizedHandler,
} from './client';

afterEach(() => setUnauthorizedHandler(null));

describe('api client', () => {
  it('parses the error envelope into an ApiError', async () => {
    server.use(
      http.post('/api/bounded-context/jobs', () =>
        apiError(409, 'duplicate_job', 'A job is already running', { details: { job_id: 'abc-123' } }),
      ),
    );
    const err = await createJob({ owner: 'acme', repo: 'shop', base_branch: 'main', model: 'auto' }).catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({
      status: 409,
      code: 'duplicate_job',
      message: 'A job is already running',
      retryable: false,
      requestId: 'req-123',
      details: { job_id: 'abc-123' },
    });
  });

  it('exposes validation field errors', async () => {
    server.use(
      http.post('/api/bounded-context/jobs', () =>
        apiError(400, 'validation_failed', 'Some fields are invalid', {
          details: { fields: { repo: 'Invalid repository name', model: 'Not a valid model identifier' } },
        }),
      ),
    );
    const err = (await createJob({ owner: 'acme', repo: '..', base_branch: 'main', model: 'x' }).catch((e) => e)) as ApiError;
    expect(err.fieldErrors).toEqual({ repo: 'Invalid repository name', model: 'Not a valid model identifier' });
  });

  it('reads Retry-After on rate limits', async () => {
    server.use(
      http.get('/api/github/repositories', () =>
        apiError(429, 'github_rate_limited', 'Slow down', { retryable: true, headers: { 'Retry-After': '30' } }),
      ),
    );
    const err = (await listRepositories({ owner: 'acme' }).catch((e) => e)) as ApiError;
    expect(err.code).toBe('github_rate_limited');
    expect(err.retryable).toBe(true);
    expect(err.retryAfter).toBe(30);
  });

  it('maps non-JSON gateway errors to a retryable internal error', async () => {
    server.use(
      http.get('/api/bounded-context/jobs/:id', () =>
        new HttpResponse('<html>Bad gateway</html>', { status: 502, headers: { 'Content-Type': 'text/html' } }),
      ),
    );
    const err = (await getJob('x').catch((e) => e)) as ApiError;
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(502);
    expect(err.code).toBe('internal');
    expect(err.retryable).toBe(true);
  });

  it('sends the CSRF header and same-origin credentials on POST, and no CSRF header on GET', async () => {
    const seen: { method: string; csrf: string | null }[] = [];
    server.use(
      http.post('/api/bounded-context/jobs/:id/cancel', ({ request }) => {
        seen.push({ method: 'POST', csrf: request.headers.get('X-Requested-With') });
        return HttpResponse.json({});
      }),
      http.get('/api/bounded-context/jobs/:id', ({ request }) => {
        seen.push({ method: 'GET', csrf: request.headers.get('X-Requested-With') });
        return HttpResponse.json({});
      }),
    );
    const fetchSpy = vi.spyOn(globalThis, 'fetch');
    await cancelJob('job-1');
    await getJob('job-1');
    expect(seen).toEqual([
      { method: 'POST', csrf: 'bcp' },
      { method: 'GET', csrf: null },
    ]);
    for (const call of fetchSpy.mock.calls) {
      expect((call[1] as RequestInit).credentials).toBe('same-origin');
    }
  });

  it('encodes path segments', async () => {
    let path = '';
    server.use(
      http.get('/api/bounded-context/jobs/:id', ({ request }) => {
        path = new URL(request.url).pathname;
        return HttpResponse.json({});
      }),
    );
    await getJob('a/b?c');
    expect(path).toBe('/api/bounded-context/jobs/a%2Fb%3Fc');
  });

  it('calls the unauthorized handler on 401 except for the session probe', async () => {
    const handler = vi.fn();
    setUnauthorizedHandler(handler);
    server.use(
      http.get('/api/auth/me', () => apiError(401, 'unauthenticated', 'Sign in')),
      http.get('/api/bounded-context/jobs/:id', () => apiError(401, 'github_auth_expired', 'Expired')),
    );
    await expect(getMe()).resolves.toBeNull();
    expect(handler).not.toHaveBeenCalled();
    const err = (await getJob('x').catch((e) => e)) as ApiError;
    expect(err.isAuthError).toBe(true);
    expect(handler).toHaveBeenCalledTimes(1);
    expect(handler.mock.calls[0][0].code).toBe('github_auth_expired');
  });

  it('parses Retry-After dates', () => {
    const now = Date.parse('2026-10-24T10:00:00Z');
    expect(parseRetryAfter('Sat, 24 Oct 2026 10:01:00 GMT', now)).toBe(60);
    expect(parseRetryAfter('nonsense', now)).toBeNull();
    expect(parseRetryAfter(null, now)).toBeNull();
  });
});
