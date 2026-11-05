import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Link, useNavigate, useParams } from 'react-router-dom';

import * as api from '../../api/client';
import { isApiError } from '../../api/client';
import { queryKeys } from '../../api/queries';
import type { Job } from '../../api/types';
import { Alert } from '../../components/Alert';
import { Button } from '../../components/Button';
import { ErrorPanel } from '../../components/ErrorPanel';
import { PageSpinner } from '../../components/Spinner';
import { StepList } from '../../components/StepList';
import { isJobLive } from '../../lib/jobState';
import { STAGE_LABELS } from '../../lib/labels';
import { buttonClass, cardClass, linkClass } from '../../lib/styles';
import { JobHeader } from './JobHeader';
import { LogViewer } from './LogViewer';
import { PublishPanel } from './PublishPanel';
import { ReviewPanel } from './ReviewPanel';
import { useJobLive, type LiveTransport } from './useJobLive';

function statusAnnouncement(job: Job): string {
  switch (job.status) {
    case 'queued':
      return 'Job queued.';
    case 'running':
      return `Running: ${STAGE_LABELS[job.stage]}.`;
    case 'succeeded':
      if (job.publish.status === 'running' || job.publish.status === 'queued') return 'Publishing pull request.';
      if (job.publish.status === 'succeeded') return 'Pull request opened.';
      if (job.publish.status === 'failed') return 'Publishing failed.';
      return 'Documentation generated. Ready for review.';
    case 'failed':
      return `Job failed: ${job.error?.message ?? 'unknown error'}.`;
    case 'cancelled':
      return 'Job cancelled.';
  }
}

function TransportNote({ transport }: { transport: LiveTransport }) {
  if (transport === 'idle') return null;
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-gray-500 dark:text-gray-400">
      <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-green-500" aria-hidden="true" />
      {transport === 'sse' ? 'Live' : 'Updating every few seconds'}
    </span>
  );
}

function JobActions({ job }: { job: Job }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const cancel = useMutation({
    mutationFn: () => api.cancelJob(job.id),
    onSuccess: (next) => qc.setQueryData(queryKeys.job(job.id), next),
  });
  const retry = useMutation({
    mutationFn: () => api.retryJob(job.id),
    onSuccess: (next) => {
      qc.setQueryData(queryKeys.job(next.id), next);
      qc.invalidateQueries({ queryKey: ['jobs', 'list'] });
      navigate(`/jobs/${encodeURIComponent(next.id)}`);
    },
  });

  return (
    <div className="flex max-w-md flex-col items-end gap-2">
      <div className="flex flex-wrap justify-end gap-2">
        {job.status === 'succeeded' && (
          <a href={api.jobArchiveUrl(job.id)} download className={buttonClass('primary')}>
            <svg viewBox="0 0 20 20" className="h-4 w-4" fill="currentColor" aria-hidden="true">
              <path d="M10 3a.75.75 0 0 1 .75.75v7.19l2.22-2.22a.75.75 0 1 1 1.06 1.06l-3.5 3.5a.75.75 0 0 1-1.06 0l-3.5-3.5a.75.75 0 1 1 1.06-1.06l2.22 2.22V3.75A.75.75 0 0 1 10 3ZM3.75 14a.75.75 0 0 1 .75.75v.5c0 .14.11.25.25.25h10.5a.25.25 0 0 0 .25-.25v-.5a.75.75 0 0 1 1.5 0v.5A1.75 1.75 0 0 1 15.25 17H4.75A1.75 1.75 0 0 1 3 15.25v-.5a.75.75 0 0 1 .75-.75Z" />
            </svg>
            Download all (.zip)
          </a>
        )}
        {job.can_retry && (
          <Button variant="secondary" onClick={() => retry.mutate()} loading={retry.isPending}>
            Retry
          </Button>
        )}
        {job.can_cancel && (
          <Button variant="danger" onClick={() => cancel.mutate()} loading={cancel.isPending}>
            Cancel job
          </Button>
        )}
      </div>
      {job.cancel_requested && job.status === 'running' && (
        <p className="text-sm text-gray-600 dark:text-gray-300" role="status">
          Cancellation requested — stopping the agent…
        </p>
      )}
      {cancel.isError && <ErrorPanel error={cancel.error} title="Could not cancel the job" />}
      {retry.isError && <ErrorPanel error={retry.error} title="Could not retry the job" />}
    </div>
  );
}

function JobDetail({ jobId }: { jobId: string }) {
  const { job, jobQuery, logs, transport } = useJobLive(jobId);

  if (jobQuery.isPending) return <PageSpinner label="Loading job…" />;
  if (!job) {
    const err = jobQuery.error;
    if (isApiError(err) && (err.code === 'not_found' || err.status === 404)) {
      return (
        <div className="space-y-3">
          <h1 className="text-xl font-semibold">Job not found</h1>
          <p className="text-sm text-gray-600 dark:text-gray-300">
            It may belong to another user or the link is wrong.
          </p>
          <Link to="/" className={linkClass}>
            Back to your jobs
          </Link>
        </div>
      );
    }
    return err ? <ErrorPanel error={err} title="Could not load the job" /> : null;
  }

  const live = isJobLive(job);
  const readOnly = job.viewer_is_owner === false;

  return (
    <div className="space-y-6">
      <nav aria-label="Breadcrumb" className="text-sm">
        <Link to={readOnly ? '/?scope=all' : '/'} className={linkClass}>
          {readOnly ? 'All jobs' : 'Jobs'}
        </Link>
        <span className="text-gray-400"> / </span>
        <span className="font-mono">{job.short_id}</span>
      </nav>

      {readOnly && (
        <Alert tone="info" title="Administrator view">
          You are viewing a job requested by <strong>{job.requested_by}</strong>. Only the requester can cancel or retry
          it; you can open its pull request on their behalf.
        </Alert>
      )}

      <p className="sr-only" aria-live="polite" aria-atomic="true">
        {statusAnnouncement(job)}
      </p>

      <div className={cardClass}>
        <JobHeader job={job} live={live} actions={<JobActions job={job} />} />
      </div>

      <section aria-labelledby="progress-heading" className={`${cardClass} space-y-5`}>
        <div className="flex items-center justify-between gap-3">
          <h2 id="progress-heading" className="text-base font-semibold">
            Progress
          </h2>
          <TransportNote transport={transport} />
        </div>
        <StepList steps={job.steps} label="Generation steps" orientation="horizontal" />
        {job.status === 'failed' && job.error && (
          <ErrorPanel error={job.error} title="Generation failed">
            {job.can_retry && <p className="text-sm">Use “Retry” to start a new job with the same settings.</p>}
          </ErrorPanel>
        )}
        {job.status === 'cancelled' && <Alert tone="info">This job was cancelled. Nothing was written to GitHub.</Alert>}
        {(job.status === 'queued' || job.status === 'running') && (
          <Alert tone="info" role="status">
            {job.status === 'queued'
              ? 'Waiting for a worker to pick up the job.'
              : `${STAGE_LABELS[job.stage]}. This can take several minutes for large repositories.`}
          </Alert>
        )}
      </section>

      {job.status === 'succeeded' && (
        <div className={`${cardClass} min-w-0`}>
          <ReviewPanel job={job} />
        </div>
      )}

      <div className={job.status === 'succeeded' ? 'grid items-start gap-6 lg:grid-cols-2' : ''}>
        <div className={`${cardClass} min-w-0`}>
          <LogViewer logs={logs} autoOpen live={live} />
        </div>
        {job.status === 'succeeded' && (
          <div className={`${cardClass} min-w-0`}>
            <PublishPanel job={job} readOnly={readOnly} />
          </div>
        )}
      </div>
    </div>
  );
}

export function JobPage() {
  const { jobId } = useParams<{ jobId: string }>();
  if (!jobId) return null;
  // Keyed so that navigating between jobs (e.g. after Retry) resets live state and logs.
  return <JobDetail key={jobId} jobId={jobId} />;
}
