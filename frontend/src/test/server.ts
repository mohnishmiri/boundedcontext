import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';

import * as f from './fixtures';

export const handlers = [
  http.get('/api/auth/me', () => HttpResponse.json({ user: f.user })),
  http.post('/api/auth/logout', () => new HttpResponse(null, { status: 204 })),
  http.get('/api/agent', () => HttpResponse.json(f.agent)),
  http.get('/api/models', () => HttpResponse.json(f.models)),
  http.get('/api/github/organizations', () => HttpResponse.json(f.organizations)),
  http.get('/api/github/repositories', () => HttpResponse.json(f.repositories)),
  http.get('/api/github/repositories/:owner/:repo', () => HttpResponse.json(f.repositoryDetail)),
  http.get('/api/github/repositories/:owner/:repo/branches', () => HttpResponse.json(f.branches)),
  http.get('/api/bounded-context/jobs', () =>
    HttpResponse.json({ items: [], page: 1, per_page: 20, has_more: false }),
  ),
  http.get('/api/bounded-context/jobs/:id', () => HttpResponse.json(f.makeJob())),
  http.get('/api/bounded-context/jobs/:id/logs', ({ request }) => {
    const after = Number(new URL(request.url).searchParams.get('after') ?? 0);
    return HttpResponse.json({ items: [], next_after: after });
  }),
  http.get('/api/bounded-context/jobs/:id/diff', () => HttpResponse.json(f.diff)),
];

export const server = setupServer(...handlers);

export function apiError(
  status: number,
  code: string,
  message: string,
  extra: { retryable?: boolean; details?: Record<string, unknown>; headers?: Record<string, string> } = {},
) {
  return HttpResponse.json(
    { error: { code, message, retryable: extra.retryable ?? false, details: extra.details, request_id: 'req-123' } },
    { status, headers: extra.headers },
  );
}
