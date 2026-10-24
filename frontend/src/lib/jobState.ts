import type { Job, JobStatus, PublishStatus } from '../api/types';

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
