import { QueryClientProvider } from '@tanstack/react-query';
import { act, renderHook, waitFor } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import type { ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Job, LogEntry } from '../../api/types';
import { createQueryClient } from '../../lib/queryClient';
import * as f from '../../test/fixtures';
import { MockEventSource } from '../../test/MockEventSource';
import { server } from '../../test/server';
import { useJobLive } from './useJobLive';

const log = (seq: number, message = `line ${seq}`): LogEntry => ({
  seq,
  ts: '2026-10-24T10:00:00Z',
  level: 'info',
  stage: 'running_agent',
  message,
});

function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={createQueryClient({ retry: false })}>{children}</QueryClientProvider>;
}

/** Serves `sequence` for successive GET /jobs/:id calls (the last entry repeats) and counts calls. */
function serveJobs(sequence: Job[]) {
  const calls = { count: 0 };
  server.use(
    http.get('/api/bounded-context/jobs/:id', () => {
      const job = sequence[Math.min(calls.count, sequence.length - 1)];
      calls.count += 1;
      return HttpResponse.json(job);
    }),
  );
  return calls;
}

/** Renders the hook and records every transport it went through (states can be shorter than a waitFor tick). */
function renderLive(pollIntervalMs?: number) {
  const transports = new Set<string>();
  const hook = renderHook(
    () => {
      const r = useJobLive(f.JOB_ID, pollIntervalMs ? { pollIntervalMs } : {});
      transports.add(r.transport);
      return r;
    },
    { wrapper },
  );
  return { ...hook, transports };
}

beforeEach(() => MockEventSource.reset());

describe('useJobLive', () => {
  it('falls back to polling when the event stream fails, and stops when the job finishes', async () => {
    vi.stubGlobal('EventSource', MockEventSource);
    const calls = serveJobs([f.runningJob(), f.runningJob(), f.succeededJob()]);
    const logCursors: number[] = [];
    server.use(
      http.get('/api/bounded-context/jobs/:id/logs', ({ request }) => {
        const after = Number(new URL(request.url).searchParams.get('after'));
        logCursors.push(after);
        // seq values are global and have gaps.
        const items = after < 120 ? [log(100), log(120)] : [];
        return HttpResponse.json({ items, next_after: items.length ? 120 : after });
      }),
    );

    const { result, transports } = renderLive(20);
    await waitFor(() => expect(result.current.transport).toBe('sse'));
    act(() => MockEventSource.latest().emit('error'));

    expect(MockEventSource.latest().closed).toBe(true);
    await waitFor(() => expect(result.current.job?.status).toBe('succeeded'));
    expect(transports.has('polling')).toBe(true);
    await waitFor(() => expect(result.current.transport).toBe('idle'));
    await waitFor(() => expect(result.current.logs.map((l) => l.seq)).toEqual([100, 120]));
    expect(logCursors).toContain(120);

    const settled = calls.count;
    await new Promise((r) => setTimeout(r, 100));
    expect(calls.count).toBe(settled);
    expect(MockEventSource.instances).toHaveLength(1);
  });

  it('closes the stream itself on a terminal job event instead of treating it as a failure', async () => {
    vi.stubGlobal('EventSource', MockEventSource);
    const calls = serveJobs([f.runningJob()]);
    const { result, transports } = renderLive(20);
    await waitFor(() => expect(result.current.transport).toBe('sse'));
    const es = MockEventSource.latest();

    act(() => es.emit('job', f.failedJob()));
    expect(es.closed).toBe(true);
    // A late error after the close (EventSource reconnect attempt) must not trigger polling.
    act(() => es.emit('error'));
    await waitFor(() => expect(result.current.transport).toBe('idle'));
    expect(transports.has('polling')).toBe(false);
    await new Promise((r) => setTimeout(r, 80));
    expect(calls.count).toBe(1);
  });

  it('merges streamed and fetched log lines in seq order without duplicates', async () => {
    vi.stubGlobal('EventSource', MockEventSource);
    serveJobs([f.runningJob()]);
    server.use(
      http.get('/api/bounded-context/jobs/:id/logs', ({ request }) => {
        const after = Number(new URL(request.url).searchParams.get('after'));
        const items = [log(5), log(9), log(17)].filter((l) => l.seq > after);
        return HttpResponse.json({ items, next_after: items.length ? items[items.length - 1].seq : after });
      }),
    );
    const { result } = renderLive();
    await waitFor(() => expect(result.current.logs).toHaveLength(3));
    await waitFor(() => expect(result.current.transport).toBe('sse'));
    const es = MockEventSource.latest();
    act(() => {
      es.emit('log', log(17));
      es.emit('log', log(30));
      es.emit('log', log(25));
    });
    expect(result.current.logs.map((l) => l.seq)).toEqual([5, 9, 17, 25, 30]);
  });

  it('polls directly when EventSource is not available', async () => {
    vi.stubGlobal('EventSource', undefined);
    const calls = serveJobs([f.runningJob(), f.succeededJob()]);
    const { result, transports } = renderLive(20);
    await waitFor(() => expect(result.current.job?.status).toBe('succeeded'));
    expect(transports.has('polling')).toBe(true);
    expect(transports.has('sse')).toBe(false);
    expect(calls.count).toBeGreaterThanOrEqual(2);
  });
});
