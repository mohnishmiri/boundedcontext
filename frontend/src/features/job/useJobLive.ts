import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useRef, useState } from 'react';

import * as api from '../../api/client';
import { queryKeys } from '../../api/queries';
import type { Job, LogEntry } from '../../api/types';
import { isJobLive } from '../../lib/jobState';

export type LiveTransport = 'sse' | 'polling' | 'idle';

export const DEFAULT_POLL_INTERVAL_MS = 3_000;
const LOG_PAGE_SIZE = 200;
const MAX_LOG_PAGES_PER_CATCH_UP = 50;
/** Lines kept in memory; older lines remain available through the API. */
const MAX_LOGS_IN_MEMORY = 5_000;

function parseEvent<T>(ev: Event): T | null {
  try {
    return JSON.parse((ev as MessageEvent<string>).data) as T;
  } catch {
    return null;
  }
}

function sseSupported(): boolean {
  return typeof EventSource !== 'undefined';
}

/**
 * Follows a job: Server-Sent Events while the job (or its publish sub-state) can still change, falling back to
 * polling `GET /jobs/{id}` + `/logs?after=` if the stream fails. Log `seq` values are only ordered (globally
 * increasing, with gaps), so they are used for ordering, de-duplication and the `after` cursor only.
 */
export function useJobLive(jobId: string, opts: { pollIntervalMs?: number } = {}) {
  const pollIntervalMs = opts.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
  const qc = useQueryClient();

  const jobQuery = useQuery({
    queryKey: queryKeys.job(jobId),
    queryFn: ({ signal }) => api.getJob(jobId, signal),
  });
  const job = jobQuery.data;
  const live = job ? isJobLive(job) : false;

  const [logs, setLogs] = useState<LogEntry[]>([]);
  const cursor = useRef(0);
  const [sseFailed, setSseFailed] = useState(false);

  const transport: LiveTransport = !live ? 'idle' : sseSupported() && !sseFailed ? 'sse' : 'polling';

  const appendLogs = useCallback((entries: LogEntry[]) => {
    if (entries.length === 0) return;
    setLogs((prev) => {
      const seen = new Set(prev.map((l) => l.seq));
      const fresh = entries.filter((e) => !seen.has(e.seq));
      if (fresh.length === 0) return prev;
      const merged = [...prev, ...fresh].sort((a, b) => a.seq - b.seq);
      return merged.length > MAX_LOGS_IN_MEMORY ? merged.slice(-MAX_LOGS_IN_MEMORY) : merged;
    });
  }, []);

  const catchUpLogs = useCallback(
    async (signal?: AbortSignal) => {
      for (let i = 0; i < MAX_LOG_PAGES_PER_CATCH_UP; i++) {
        const page = await api.getJobLogs(jobId, cursor.current, LOG_PAGE_SIZE, signal);
        appendLogs(page.items);
        const last = page.items.length > 0 ? page.items[page.items.length - 1].seq : cursor.current;
        cursor.current = Math.max(cursor.current, page.next_after, last);
        if (page.items.length < LOG_PAGE_SIZE) break;
      }
    },
    [jobId, appendLogs],
  );

  // Initial log backlog.
  useEffect(() => {
    const ac = new AbortController();
    catchUpLogs(ac.signal).catch(() => undefined);
    return () => ac.abort();
  }, [catchUpLogs]);

  // Final catch-up once the job stops changing (the stream or poll may end before the last lines are read).
  const wasLive = useRef(live);
  useEffect(() => {
    if (wasLive.current && !live) {
      catchUpLogs().catch(() => undefined);
      qc.invalidateQueries({ queryKey: ['jobs', 'list'] });
    }
    wasLive.current = live;
  }, [live, catchUpLogs, qc]);

  // Server-Sent Events.
  useEffect(() => {
    if (transport !== 'sse') return;
    const key = queryKeys.job(jobId);
    const es = new EventSource(api.jobEventsUrl(jobId));
    let closed = false;
    const close = () => {
      closed = true;
      es.close();
    };
    es.addEventListener('open', () => {
      // Lines written between the backlog fetch and the stream connecting.
      catchUpLogs().catch(() => undefined);
    });
    es.addEventListener('job', (ev) => {
      const next = parseEvent<Job>(ev);
      if (!next) return;
      qc.setQueryData(key, next);
      // The server closes the stream in this state; close first so EventSource does not reconnect.
      if (!isJobLive(next)) close();
    });
    es.addEventListener('log', (ev) => {
      const entry = parseEvent<LogEntry>(ev);
      if (entry) {
        appendLogs([entry]);
      }
    });
    es.addEventListener('error', () => {
      if (closed) return;
      close();
      const current = qc.getQueryData<Job>(key);
      if (current && !isJobLive(current)) return;
      setSseFailed(true);
    });
    return close;
  }, [transport, jobId, qc, appendLogs, catchUpLogs]);

  // Polling fallback.
  useEffect(() => {
    if (transport !== 'polling') return;
    const key = queryKeys.job(jobId);
    const ac = new AbortController();
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      try {
        const next = await api.getJob(jobId, ac.signal);
        if (stopped) return;
        qc.setQueryData(key, next);
        await catchUpLogs(ac.signal);
      } catch {
        // Transient failures: keep polling. 401s are handled globally (sign-in screen).
      }
      if (!stopped) timer = setTimeout(tick, pollIntervalMs);
    };
    timer = setTimeout(tick, pollIntervalMs);
    return () => {
      stopped = true;
      clearTimeout(timer);
      ac.abort();
    };
  }, [transport, jobId, qc, catchUpLogs, pollIntervalMs]);

  return { job, jobQuery, logs, transport };
}
