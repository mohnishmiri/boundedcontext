import type { JobStatus, PublishStatus } from '../api/types';

const jobStyles: Record<JobStatus, string> = {
  queued: 'bg-gray-100 text-gray-700 ring-gray-300 dark:bg-gray-800 dark:text-gray-200 dark:ring-gray-600',
  running: 'bg-blue-50 text-blue-700 ring-blue-300 dark:bg-blue-950 dark:text-blue-200 dark:ring-blue-800',
  succeeded: 'bg-green-50 text-green-700 ring-green-300 dark:bg-green-950 dark:text-green-200 dark:ring-green-800',
  failed: 'bg-red-50 text-red-700 ring-red-300 dark:bg-red-950 dark:text-red-200 dark:ring-red-800',
  cancelled: 'bg-gray-100 text-gray-600 ring-gray-300 dark:bg-gray-800 dark:text-gray-300 dark:ring-gray-600',
};

const jobLabels: Record<JobStatus, string> = {
  queued: 'Queued',
  running: 'Running',
  succeeded: 'Succeeded',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

const publishStyles: Record<PublishStatus, string> = {
  not_started: jobStyles.cancelled,
  queued: jobStyles.queued,
  running: jobStyles.running,
  succeeded: jobStyles.succeeded,
  failed: jobStyles.failed,
};

const publishLabels: Record<PublishStatus, string> = {
  not_started: 'Not published',
  queued: 'PR queued',
  running: 'Publishing',
  succeeded: 'PR opened',
  failed: 'Publish failed',
};

const pill = 'inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset';

export function StatusBadge({ status }: { status: JobStatus }) {
  return (
    <span className={`${pill} ${jobStyles[status]}`}>
      {status === 'running' && <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-current" aria-hidden="true" />}
      {jobLabels[status]}
    </span>
  );
}

export function PublishBadge({ status }: { status: PublishStatus }) {
  return <span className={`${pill} ${publishStyles[status]}`}>{publishLabels[status]}</span>;
}
