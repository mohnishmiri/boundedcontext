import type {
  AgentInfo,
  ApiErrorBody,
  BranchPage,
  CreateJobRequest,
  ErrorCode,
  Job,
  JobDiff,
  JobStatus,
  JobSummary,
  LogPage,
  ModelList,
  OrganizationList,
  Page,
  Repository,
  RepositoryDetail,
  User,
} from './types';

const API_BASE = '/api';

/** Header the API requires on every mutating request (CSRF defence together with the Origin check). */
export const CSRF_HEADER = 'X-Requested-With';
export const CSRF_VALUE = 'bcp';

export class ApiError extends Error {
  readonly status: number;
  readonly code: ErrorCode;
  readonly retryable: boolean;
  readonly details: Record<string, unknown> | undefined;
  readonly requestId: string | null;
  /** Seconds to wait before retrying, from the `Retry-After` header. */
  readonly retryAfter: number | null;

  constructor(init: {
    status: number;
    code: ErrorCode;
    message: string;
    retryable: boolean;
    details?: Record<string, unknown>;
    requestId?: string | null;
    retryAfter?: number | null;
  }) {
    super(init.message);
    this.name = 'ApiError';
    this.status = init.status;
    this.code = init.code;
    this.retryable = init.retryable;
    this.details = init.details;
    this.requestId = init.requestId ?? null;
    this.retryAfter = init.retryAfter ?? null;
  }

  /** Per-field messages of a `validation_failed` error (`details.fields`). */
  get fieldErrors(): Record<string, string> {
    const fields = this.details?.fields;
    if (!fields || typeof fields !== 'object') return {};
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(fields as Record<string, unknown>)) {
      if (typeof v === 'string') out[k] = v;
    }
    return out;
  }

  get isAuthError(): boolean {
    return this.status === 401 && (this.code === 'unauthenticated' || this.code === 'github_auth_expired');
  }
}

export function isApiError(err: unknown): err is ApiError {
  return err instanceof ApiError;
}

type UnauthorizedHandler = (err: ApiError) => void;
let unauthorizedHandler: UnauthorizedHandler | null = null;

/** Registers the callback invoked when any request (except the session probe) gets a 401. */
export function setUnauthorizedHandler(handler: UnauthorizedHandler | null): void {
  unauthorizedHandler = handler;
}

export function parseRetryAfter(value: string | null, now: number = Date.now()): number | null {
  if (!value) return null;
  const trimmed = value.trim();
  if (/^\d+$/.test(trimmed)) return Number(trimmed);
  const date = Date.parse(trimmed);
  if (Number.isNaN(date)) return null;
  return Math.max(0, Math.ceil((date - now) / 1000));
}

function fallbackCode(status: number): { code: ErrorCode; retryable: boolean; message: string } {
  switch (status) {
    case 400:
      return { code: 'validation_failed', retryable: false, message: 'The request was rejected.' };
    case 401:
      return { code: 'unauthenticated', retryable: false, message: 'Please sign in.' };
    case 403:
      return { code: 'forbidden', retryable: false, message: 'You are not allowed to do that.' };
    case 404:
      return { code: 'not_found', retryable: false, message: 'Not found.' };
    case 409:
      return { code: 'conflict', retryable: false, message: 'The request conflicts with the current state.' };
    case 413:
      return { code: 'payload_too_large', retryable: false, message: 'The request is too large.' };
    case 429:
      return { code: 'rate_limited', retryable: true, message: 'Too many requests. Please wait and try again.' };
    default:
      return {
        code: 'internal',
        retryable: status >= 500,
        message:
          status >= 500 ? 'The service is temporarily unavailable. Please try again.' : `Unexpected response (${status}).`,
      };
  }
}

async function toApiError(res: Response): Promise<ApiError> {
  const retryAfter = parseRetryAfter(res.headers.get('Retry-After'));
  const headerRequestId = res.headers.get('X-Request-Id');
  let body: Partial<ApiErrorBody> | null = null;
  if ((res.headers.get('Content-Type') ?? '').includes('application/json')) {
    body = (await res.json().catch(() => null)) as Partial<ApiErrorBody> | null;
  }
  const e = body?.error;
  if (e && typeof e.code === 'string' && typeof e.message === 'string') {
    return new ApiError({
      status: res.status,
      code: e.code,
      message: e.message,
      retryable: Boolean(e.retryable),
      details: e.details,
      requestId: e.request_id ?? headerRequestId,
      retryAfter,
    });
  }
  return new ApiError({ status: res.status, ...fallbackCode(res.status), requestId: headerRequestId, retryAfter });
}

type RequestOptions = {
  method?: 'GET' | 'POST';
  query?: Record<string, string | number | undefined | null>;
  body?: unknown;
  /** Do not invoke the unauthorized handler (used by the session probe). */
  allowUnauthenticated?: boolean;
  signal?: AbortSignal;
};

export function buildUrl(path: string, query?: RequestOptions['query']): string {
  const params = new URLSearchParams();
  for (const [k, v] of Object.entries(query ?? {})) {
    if (v !== undefined && v !== null && v !== '') params.set(k, String(v));
  }
  const qs = params.toString();
  return `${API_BASE}${path}${qs ? `?${qs}` : ''}`;
}

async function send(path: string, opts: RequestOptions = {}): Promise<Response> {
  const method = opts.method ?? 'GET';
  const headers: Record<string, string> = { Accept: 'application/json' };
  if (method !== 'GET') headers[CSRF_HEADER] = CSRF_VALUE;
  let body: string | undefined;
  if (opts.body !== undefined) {
    headers['Content-Type'] = 'application/json';
    body = JSON.stringify(opts.body);
  }
  let res: Response;
  try {
    res = await fetch(buildUrl(path, opts.query), {
      method,
      headers,
      body,
      credentials: 'same-origin',
      signal: opts.signal,
    });
  } catch (err) {
    if (err instanceof DOMException && err.name === 'AbortError') throw err;
    throw new ApiError({
      status: 0,
      code: 'internal',
      retryable: true,
      message: 'Could not reach the portal. Check your connection and try again.',
    });
  }
  if (!res.ok) {
    const err = await toApiError(res);
    if (err.status === 401 && !opts.allowUnauthenticated) unauthorizedHandler?.(err);
    throw err;
  }
  return res;
}

async function request<T>(path: string, opts: RequestOptions = {}): Promise<T> {
  const res = await send(path, opts);
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

const seg = encodeURIComponent;

// ---------- auth ----------

export function signInUrl(returnTo: string): string {
  return buildUrl('/auth/github', { return_to: returnTo });
}

export async function getMe(signal?: AbortSignal): Promise<User | null> {
  try {
    const res = await request<{ user: User }>('/auth/me', { allowUnauthenticated: true, signal });
    return res.user;
  } catch (err) {
    if (isApiError(err) && err.status === 401) return null;
    throw err;
  }
}

export function logout(): Promise<void> {
  return request<void>('/auth/logout', { method: 'POST', allowUnauthenticated: true });
}

// ---------- GitHub browsing ----------

export function listOrganizations(signal?: AbortSignal): Promise<OrganizationList> {
  return request('/github/organizations', { signal });
}

export function listRepositories(
  params: { owner: string; q?: string; page?: number; per_page?: number },
  signal?: AbortSignal,
): Promise<Page<Repository>> {
  return request('/github/repositories', { query: params, signal });
}

export function getRepository(owner: string, repo: string, signal?: AbortSignal): Promise<RepositoryDetail> {
  return request(`/github/repositories/${seg(owner)}/${seg(repo)}`, { signal });
}

export function listBranches(
  owner: string,
  repo: string,
  params: { q?: string; page?: number; per_page?: number },
  signal?: AbortSignal,
): Promise<BranchPage> {
  return request(`/github/repositories/${seg(owner)}/${seg(repo)}/branches`, { query: params, signal });
}

// ---------- agent and models ----------

export function getAgent(signal?: AbortSignal): Promise<AgentInfo> {
  return request('/agent', { signal });
}

export function getModels(signal?: AbortSignal): Promise<ModelList> {
  return request('/models', { signal });
}

// ---------- jobs ----------

const JOBS = '/bounded-context/jobs';

export function createJob(body: CreateJobRequest): Promise<Job> {
  return request(JOBS, { method: 'POST', body });
}

export function listJobs(
  params: { status?: JobStatus; owner?: string; repo?: string; page?: number; per_page?: number },
  signal?: AbortSignal,
): Promise<Page<JobSummary>> {
  return request(JOBS, { query: params, signal });
}

export function getJob(id: string, signal?: AbortSignal): Promise<Job> {
  return request(`${JOBS}/${seg(id)}`, { signal });
}

export function getJobLogs(id: string, after: number, limit = 200, signal?: AbortSignal): Promise<LogPage> {
  return request(`${JOBS}/${seg(id)}/logs`, { query: { after, limit }, signal });
}

export function getJobDiff(id: string, signal?: AbortSignal): Promise<JobDiff> {
  return request(`${JOBS}/${seg(id)}/diff`, { signal });
}

export function requestPullRequest(id: string): Promise<Job> {
  return request(`${JOBS}/${seg(id)}/pull-request`, { method: 'POST' });
}

export function cancelJob(id: string): Promise<Job> {
  return request(`${JOBS}/${seg(id)}/cancel`, { method: 'POST' });
}

export function retryJob(id: string): Promise<Job> {
  return request(`${JOBS}/${seg(id)}/retry`, { method: 'POST' });
}

export function jobOutputUrl(id: string): string {
  return buildUrl(`${JOBS}/${seg(id)}/output`);
}

export function jobEventsUrl(id: string): string {
  return buildUrl(`${JOBS}/${seg(id)}/events`);
}
