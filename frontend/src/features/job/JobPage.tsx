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
    <div className="space-y-3">
      <div className="flex flex-wrap gap-2">
        {job.can_cancel && (
          <Button variant="danger" onClick={() => cancel.mutate()} loading={cancel.isPending}>
            Cancel job
          </Button>
        )}
        {job.can_retry && (
          <Button variant="secondary" onClick={() => retry.mutate()} loading={retry.isPending}>
            Retry
          </Button>
        )}
        {job.status === 'succeeded' && (
          <a href={api.jobOutputUrl(job.id)} download className={buttonClass('secondary')}>
            Download output
          </a>
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

  return (
    <div className="space-y-6">
      <nav aria-label="Breadcrumb" className="text-sm">
        <Link to="/" className={linkClass}>
          Jobs
        </Link>
        <span className="text-gray-400"> / </span>
        <span className="font-mono">{job.short_id}</span>
      </nav>

      <p className="sr-only" aria-live="polite" aria-atomic="true">
        {statusAnnouncement(job)}
      </p>

      <div className={cardClass}>
        <JobHeader job={job} live={live} />
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        <div className={`${cardClass} space-y-4 lg:col-span-1`}>
          <div className="flex items-center justify-between">
            <h2 className="text-base font-semibold">Progress</h2>
            <TransportNote transport={transport} />
          </div>
          <StepList steps={job.steps} label="Generation steps" />
          <JobActions job={job} />
        </div>

        <div className="space-y-6 lg:col-span-2">
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
          {job.status === 'succeeded' && (
            <>
              <div className={cardClass}>
                <ReviewPanel job={job} />
              </div>
              <div className={cardClass}>
                <PublishPanel job={job} />
              </div>
            </>
          )}
          <div className={cardClass}>
            <LogViewer logs={logs} autoOpen={job.status === 'failed'} />
          </div>
        </div>
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
