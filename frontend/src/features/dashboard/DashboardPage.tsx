import { Link, useSearchParams } from 'react-router-dom';

import { useJobList } from '../../api/queries';
import type { JobStatus } from '../../api/types';
import { ErrorPanel } from '../../components/ErrorPanel';
import { Pagination } from '../../components/Pagination';
import { Spinner } from '../../components/Spinner';
import { PublishBadge, StatusBadge } from '../../components/StatusBadge';
import { formatDateTime, safeExternalUrl } from '../../lib/format';
import { JOB_STATUSES, parseJobStatus, STAGE_LABELS } from '../../lib/labels';
import { buttonClass, cardClass, inputClass, linkClass } from '../../lib/styles';

const statusLabels: Record<JobStatus, string> = {
  queued: 'Queued',
  running: 'Running',
  succeeded: 'Succeeded',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

export function DashboardPage() {
  const [params, setParams] = useSearchParams();
  const status = parseJobStatus(params.get('status'));
  const page = Math.max(1, Number.parseInt(params.get('page') ?? '1', 10) || 1);
  const jobs = useJobList(status, page);

  const update = (next: { status?: JobStatus; page?: number }) => {
    const p = new URLSearchParams();
    if (next.status) p.set('status', next.status);
    if (next.page && next.page > 1) p.set('page', String(next.page));
    setParams(p);
  };

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold">Your generations</h1>
          <p className="text-sm text-gray-600 dark:text-gray-300">
            Bounded-context documentation jobs you started. Only you can see them.
          </p>
        </div>
        <Link to="/jobs/new" className={buttonClass('primary')}>
          New generation
        </Link>
      </div>

      <div className={cardClass}>
        <div className="mb-4 flex flex-wrap items-center gap-3">
          <label htmlFor="status-filter" className="text-sm font-medium">
            Status
          </label>
          <select
            id="status-filter"
            className={`${inputClass} w-44`}
            value={status ?? ''}
            onChange={(e) => update({ status: parseJobStatus(e.target.value), page: 1 })}
          >
            <option value="">All</option>
            {JOB_STATUSES.map((s) => (
              <option key={s} value={s}>
                {statusLabels[s]}
              </option>
            ))}
          </select>
          {jobs.isFetching && <Spinner size="sm" label="Refreshing" />}
        </div>

        {jobs.isPending && <Spinner label="Loading jobs…" />}
        {jobs.isError && <ErrorPanel error={jobs.error} title="Could not load jobs" />}
        {jobs.data && jobs.data.items.length === 0 && (
          <div className="py-10 text-center">
            <p className="text-sm text-gray-600 dark:text-gray-300">
              {status ? 'No jobs with this status.' : 'You have not generated any documentation yet.'}
            </p>
            {!status && (
              <Link to="/jobs/new" className={`${linkClass} mt-2 inline-block`}>
                Start your first generation
              </Link>
            )}
          </div>
        )}
        {jobs.data && jobs.data.items.length > 0 && (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead className="border-b border-gray-200 text-xs uppercase text-gray-500 dark:border-gray-700 dark:text-gray-400">
                <tr>
                  <th scope="col" className="py-2 pr-4 font-medium">Repository</th>
                  <th scope="col" className="py-2 pr-4 font-medium">Branch</th>
                  <th scope="col" className="py-2 pr-4 font-medium">Model</th>
                  <th scope="col" className="py-2 pr-4 font-medium">Status</th>
                  <th scope="col" className="py-2 pr-4 font-medium">Pull request</th>
                  <th scope="col" className="py-2 font-medium">Started</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100 dark:divide-gray-800">
                {jobs.data.items.map((job) => {
                  const pr = safeExternalUrl(job.pr_url);
                  return (
                    <tr key={job.id}>
                      <td className="py-3 pr-4">
                        <Link to={`/jobs/${encodeURIComponent(job.id)}`} className={linkClass}>
                          {job.owner}/{job.repo}
                        </Link>
                        <span className="ml-2 font-mono text-xs text-gray-500">{job.short_id}</span>
                        {job.batch_id && (
                          <Link
                            to={`/batches/${encodeURIComponent(job.batch_id)}`}
                            className={`${linkClass} ml-2 text-xs`}
                            aria-label={`Batch of ${job.owner}/${job.repo}`}
                          >
                            Batch
                          </Link>
                        )}
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs">{job.base_branch}</td>
                      <td className="py-3 pr-4 text-xs">{job.model}</td>
                      <td className="py-3 pr-4">
                        <StatusBadge status={job.status} />
                        {job.status === 'running' && (
                          <span className="ml-2 text-xs text-gray-500">{STAGE_LABELS[job.stage]}</span>
                        )}
                      </td>
                      <td className="py-3 pr-4">
                        {pr ? (
                          <a href={pr} target="_blank" rel="noopener noreferrer" className={linkClass}>
                            View PR
                          </a>
                        ) : job.status === 'succeeded' ? (
                          <PublishBadge status={job.publish_status} />
                        ) : (
                          <span className="text-gray-400">—</span>
                        )}
                      </td>
                      <td className="py-3 text-xs text-gray-600 dark:text-gray-300">{formatDateTime(job.created_at)}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
        {jobs.data && (
          <Pagination
            label="Job list pages"
            page={page}
            hasMore={jobs.data.has_more}
            onPageChange={(p) => update({ status, page: p })}
            disabled={jobs.isFetching}
          />
        )}
      </div>
    </div>
  );
}
