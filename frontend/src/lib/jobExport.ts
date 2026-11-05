import type { JobSummary } from '../api/types';
import { formatDateTime, formatDuration } from './format';
import { elapsedMs } from './jobState';
import { STAGE_LABELS } from './labels';
import type { Cell } from './xlsx';

const HEADER = [
  'Repository',
  'Job ID',
  'Requested by',
  'Branch',
  'Model',
  'Status',
  'Stage',
  'Publish status',
  'Pull request',
  'Duration',
  'Duration (seconds)',
  'Created',
  'Started',
  'Finished',
  'Batch ID',
  'Link',
];

/** Spreadsheet rows (header first) for a list of jobs. */
export function jobRows(jobs: JobSummary[], now = Date.now()): Cell[][] {
  const origin = typeof window === 'undefined' ? '' : window.location.origin;
  return [
    HEADER,
    ...jobs.map((j) => {
      const ms = elapsedMs(j, now);
      return [
        `${j.owner}/${j.repo}`,
        j.id,
        j.requested_by,
        j.base_branch,
        j.model,
        j.status,
        j.status === 'running' ? STAGE_LABELS[j.stage] : '',
        j.status === 'succeeded' ? j.publish_status : '',
        j.pr_url ?? '',
        ms === null ? '' : formatDuration(ms),
        ms === null ? null : Math.round(ms / 1000),
        formatDateTime(j.created_at),
        j.started_at ? formatDateTime(j.started_at) : '',
        j.finished_at ? formatDateTime(j.finished_at) : '',
        j.batch_id ?? '',
        `${origin}/jobs/${encodeURIComponent(j.id)}`,
      ];
    }),
  ];
}
