import { useState } from 'react';
import { Link, useLocation, useParams } from 'react-router-dom';

import { useBatchJobs } from '../../api/queries';
import type { BatchFailure } from '../../api/types';
import { Alert } from '../../components/Alert';
import { ErrorPanel } from '../../components/ErrorPanel';
import { ExportButton } from '../../components/ExportButton';
import { Pagination } from '../../components/Pagination';
import { PageSpinner } from '../../components/Spinner';
import { PublishBadge, StatusBadge } from '../../components/StatusBadge';
import { formatDateTime } from '../../lib/format';
import { jobRows } from '../../lib/jobExport';
import { isTerminalStatus } from '../../lib/jobState';
import { STAGE_LABELS } from '../../lib/labels';
import { cardClass, inputClass, linkClass } from '../../lib/styles';
import { parseFailures, readBatchFailures } from './batchFailures';

const BATCH_ROWS_PER_PAGE = 10;

const JOB_ID_RE = /^[0-9a-fA-F-]{8,64}$/;

function existingJobId(failure: BatchFailure): string | null {
  const details = failure.error.details;
  if (failure.error.code !== 'duplicate_job' || !details || typeof details !== 'object') return null;
  const id = (details as Record<string, unknown>).job_id;
  return typeof id === 'string' && JOB_ID_RE.test(id) ? id : null;
}

function FailureList({ failures }: { failures: BatchFailure[] }) {
  return (
    <section className={`${cardClass} space-y-3`} aria-labelledby="batch-failures">
      <h2 id="batch-failures" className="text-base font-semibold">
        Not started ({failures.length})
      </h2>
      <p className="text-sm text-gray-600 dark:text-gray-300">
        These repositories were rejected before a job was created. Nothing ran for them.
      </p>
      <ul className="divide-y divide-gray-100 rounded-md border border-gray-200 dark:divide-gray-800 dark:border-gray-700">
        {failures.map((f) => {
          const existing = existingJobId(f);
          return (
            <li key={`${f.owner}/${f.repo}@${f.base_branch}`} className="space-y-1 px-3 py-2 text-sm">
              <p>
                <span className="font-medium">
                  {f.owner}/{f.repo}
                </span>{' '}
                <code className="font-mono text-xs text-gray-500">{f.base_branch}</code>
              </p>
              <p className="text-red-700 dark:text-red-300">{f.error.message}</p>
              <p className="text-xs text-gray-500 dark:text-gray-400">code: {f.error.code}</p>
              {existing && (
                <Link to={`/jobs/${encodeURIComponent(existing)}`} className={linkClass}>
                  Open the existing job
                </Link>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function BatchDetail({ batchId }: { batchId: string }) {
  const location = useLocation();
  const jobs = useBatchJobs(batchId);
  const stateFailures = parseFailures((location.state as { failures?: unknown } | null)?.failures);
  const failures = stateFailures.length > 0 ? stateFailures : readBatchFailures(batchId);
  const [query, setQuery] = useState('');
  const [page, setPage] = useState(1);

  if (jobs.isPending) return <PageSpinner label="Loading batch…" />;
  if (!jobs.data) {
    return jobs.error ? <ErrorPanel error={jobs.error} title="Could not load the batch" /> : null;
  }

  const items = jobs.data.items;
  const done = items.filter((j) => isTerminalStatus(j.status)).length;
  const needle = query.trim().toLowerCase();
  const matching = needle
    ? items.filter((j) =>
        [`${j.owner}/${j.repo}`, j.base_branch, j.short_id, j.status].some((v) => v.toLowerCase().includes(needle)),
      )
    : items;
  const pageCount = Math.max(1, Math.ceil(matching.length / BATCH_ROWS_PER_PAGE));
  const current = Math.min(page, pageCount);
  const rows = matching.slice((current - 1) * BATCH_ROWS_PER_PAGE, current * BATCH_ROWS_PER_PAGE);

  return (
    <div className="space-y-6">
      <nav aria-label="Breadcrumb" className="text-sm">
        <Link to="/" className={linkClass}>
          Jobs
        </Link>
        <span className="text-gray-400"> / </span>
        <span>Batch</span> <span className="font-mono">{batchId.slice(0, 8)}</span>
      </nav>

      <div>
        <h1 className="text-2xl font-semibold">Batch generation</h1>
        <p className="text-sm text-gray-600 dark:text-gray-300" role="status">
          {items.length === 0
            ? 'No jobs were created for this batch.'
            : `${done} of ${items.length} ${items.length === 1 ? 'job' : 'jobs'} finished.`}
        </p>
      </div>

      {jobs.isError && <ErrorPanel error={jobs.error} title="Could not refresh the batch" />}
      {jobs.data.has_more && (
        <Alert tone="info">Only the first {items.length} jobs of this batch are listed.</Alert>
      )}

      {items.length > 0 && (
        <section className={cardClass} aria-labelledby="batch-jobs">
          <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
            <h2 id="batch-jobs" className="text-base font-semibold">
              Jobs
            </h2>
            <div className="flex flex-wrap items-center gap-3">
              <label htmlFor="batch-search" className="sr-only">
                Search jobs in this batch
              </label>
              <input
                id="batch-search"
                type="search"
                value={query}
                onChange={(e) => {
                  setQuery(e.target.value);
                  setPage(1);
                }}
                placeholder="Search repository, branch, id or status"
                autoComplete="off"
                spellCheck={false}
                className={`${inputClass} w-72`}
              />
              <ExportButton
                fileBase={`batch-${batchId.slice(0, 8)}`}
                build={() => [{ name: 'Batch jobs', rows: jobRows(matching) }]}
              />
            </div>
          </div>
          {rows.length === 0 && (
            <p className="py-6 text-center text-sm text-gray-600 dark:text-gray-300">No jobs match this search.</p>
          )}
          {rows.length > 0 && (
            <div className="overflow-x-auto">
              <table className="w-full text-left text-sm">
                <thead className="border-b border-gray-200 text-xs uppercase text-gray-500 dark:border-gray-700 dark:text-gray-400">
                  <tr>
                    <th scope="col" className="py-2 pr-4 font-medium">Repository</th>
                    <th scope="col" className="py-2 pr-4 font-medium">Branch</th>
                    <th scope="col" className="py-2 pr-4 font-medium">Status</th>
                    <th scope="col" className="py-2 pr-4 font-medium">Pull request</th>
                    <th scope="col" className="py-2 font-medium">Started</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-100 dark:divide-gray-800">
                  {rows.map((job) => (
                    <tr key={job.id}>
                      <td className="py-3 pr-4">
                        <Link to={`/jobs/${encodeURIComponent(job.id)}`} className={linkClass}>
                          {job.owner}/{job.repo}
                        </Link>
                        <span className="ml-2 font-mono text-xs text-gray-500">{job.short_id}</span>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs">{job.base_branch}</td>
                      <td className="py-3 pr-4">
                        <StatusBadge status={job.status} />
                        {job.status === 'running' && (
                          <span className="ml-2 text-xs text-gray-500">{STAGE_LABELS[job.stage]}</span>
                        )}
                      </td>
                      <td className="py-3 pr-4">
                        {job.status === 'succeeded' ? (
                          <PublishBadge status={job.publish_status} />
                        ) : (
                          <span className="text-gray-400">—</span>
                        )}
                      </td>
                      <td className="py-3 text-xs text-gray-600 dark:text-gray-300">{formatDateTime(job.created_at)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          <Pagination
            label="Batch job pages"
            page={current}
            hasMore={current < pageCount}
            total={matching.length}
            perPage={BATCH_ROWS_PER_PAGE}
            onPageChange={setPage}
          />
        </section>
      )}

      {failures.length > 0 && <FailureList failures={failures} />}
    </div>
  );
}

export function BatchPage() {
  const { batchId } = useParams<{ batchId: string }>();
  if (!batchId) return null;
  return <BatchDetail key={batchId} batchId={batchId} />;
}
