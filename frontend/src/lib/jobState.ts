import type { Job, JobStatus, JobSummary, PublishStatus } from '../api/types';

export function isTerminalStatus(status: JobStatus): boolean {
  return status === 'succeeded' || status === 'failed' || status === 'cancelled';
}

export function isPublishActive(status: PublishStatus): boolean {
  return status === 'queued' || status === 'running';
}

/**
 * Whether the job can still change on its own. Mirrors the server's rule for closing the event stream: the job
 * is terminal AND publishing is not queued/running.
 */
export function isJobLive(job: Pick<Job, 'status' | 'publish'>): boolean {
  return !isTerminalStatus(job.status) || isPublishActive(job.publish.status);
}

/**
 * Run time in ms: up to `now` while running (so the UI can tick every second), else start to finish.
 * `null` before a worker has started the job.
 */
export function elapsedMs(
  job: Pick<JobSummary, 'status' | 'started_at' | 'finished_at'>,
  now: number,
): number | null {
  if (!job.started_at) return null;
  const start = new Date(job.started_at).getTime();
  if (Number.isNaN(start)) return null;
  if (job.status === 'running') return Math.max(0, now - start);
  const end = job.finished_at ? new Date(job.finished_at).getTime() : Number.NaN;
  return Number.isNaN(end) ? null : Math.max(0, end - start);
}
