import type { GenerationStage, JobStatus } from '../api/types';

export const STAGE_LABELS: Record<GenerationStage, string> = {
  queued: 'Waiting for a worker',
  preparing: 'Checking access and resolving branch',
  fetching_repository: 'Fetching repository snapshot',
  running_agent: 'Running bounded-context agent',
  validating_output: 'Validating generated documentation',
  completed: 'Completed',
};

export const JOB_STATUSES: JobStatus[] = ['queued', 'running', 'succeeded', 'failed', 'cancelled'];

export function parseJobStatus(value: string | null): JobStatus | undefined {
  return JOB_STATUSES.find((s) => s === value);
}
