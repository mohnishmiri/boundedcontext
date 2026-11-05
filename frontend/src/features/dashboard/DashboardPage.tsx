import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';

import { listAllJobs } from '../../api/client';
import { JOBS_PER_PAGE, JOBS_PER_PAGE_OPTIONS, useJobList, useMe } from '../../api/queries';
import type { JobScope, JobStatus, JobStatusCounts, JobSummary } from '../../api/types';
import { ErrorPanel } from '../../components/ErrorPanel';
import { ExportButton } from '../../components/ExportButton';
import { Pagination } from '../../components/Pagination';
import { Spinner } from '../../components/Spinner';
import { PublishBadge, StatusBadge } from '../../components/StatusBadge';
import { formatDateTime, formatDuration, safeExternalUrl } from '../../lib/format';
import { useDebouncedValue, useNow } from '../../lib/hooks';
import { jobRows } from '../../lib/jobExport';
import { elapsedMs } from '../../lib/jobState';
import { parseJobStatus, STAGE_LABELS } from '../../lib/labels';
import { buttonClass, cardClass, inputClass, linkClass } from '../../lib/styles';

type Tile = { key: JobStatus | 'all'; label: string; accent: string; active: string };

/** Filter tiles, in the order users scan them: everything, what is in flight, then outcomes. */
const TILES: Tile[] = [
  {
    key: 'all',
    label: 'All',
    accent: 'bg-indigo-500',
    active: 'ring-indigo-500 bg-indigo-50 dark:bg-indigo-950/60',
  },
  {
    key: 'running',
    label: 'Running',
    accent: 'bg-blue-500',
    active: 'ring-blue-500 bg-blue-50 dark:bg-blue-950/60',
  },
  {
    key: 'queued',
    label: 'Queued',
    accent: 'bg-gray-400',
    active: 'ring-gray-500 bg-gray-100 dark:bg-gray-800',
  },
  {
    key: 'succeeded',
    label: 'Succeeded',
    accent: 'bg-green-500',
    active: 'ring-green-500 bg-green-50 dark:bg-green-950/60',
  },
  {
    key: 'failed',
    label: 'Failed',
    accent: 'bg-red-500',
    active: 'ring-red-500 bg-red-50 dark:bg-red-950/60',
  },
  {
    key: 'cancelled',
    label: 'Cancelled',
    accent: 'bg-gray-300',
    active: 'ring-gray-400 bg-gray-100 dark:bg-gray-800',
  },
];

const countFormat = new Intl.NumberFormat();

function StatusTiles({
  selected,
  counts,
  onSelect,
}: {
  selected: JobStatus | undefined;
  counts: JobStatusCounts | undefined;
  onSelect: (status: JobStatus | undefined) => void;
}) {
  return (
    <div role="group" aria-label="Filter by status" className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
      {TILES.map((t) => {
        const isSelected = (selected ?? 'all') === t.key;
        const count = counts?.[t.key];
        return (
          <button
            key={t.key}
            type="button"
            aria-pressed={isSelected}
            onClick={() => onSelect(t.key === 'all' ? undefined : t.key)}
            className={`group relative flex flex-col items-start overflow-hidden rounded-lg px-4 py-3 text-left ring-1 ring-inset transition focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-500 ${
              isSelected
                ? `${t.active} ring-2`
                : 'bg-white ring-gray-200 hover:bg-gray-50 hover:ring-gray-300 dark:bg-gray-900 dark:ring-gray-700 dark:hover:bg-gray-800'
            }`}
          >
            <span className={`absolute inset-x-0 top-0 h-1 ${t.accent}`} aria-hidden="true" />
            <span className="text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
              {t.label}
            </span>
            <span className="mt-1 text-2xl font-semibold tabular-nums text-gray-900 dark:text-white">
              {count === undefined ? '–' : countFormat.format(count)}
            </span>
            {t.key === 'running' && (count ?? 0) > 0 && (
              <span className="absolute right-3 top-3 h-2 w-2 animate-pulse rounded-full bg-blue-500" aria-hidden="true" />
            )}
          </button>
        );
      })}
    </div>
  );
}

function ScopeSwitch({ scope, onChange }: { scope: JobScope; onChange: (s: JobScope) => void }) {
  const options: { key: JobScope; label: string }[] = [
    { key: 'mine', label: 'My jobs' },
    { key: 'all', label: 'All users' },
  ];
  return (
    <div role="group" aria-label="Whose jobs" className="inline-flex rounded-md shadow-sm ring-1 ring-inset ring-gray-300 dark:ring-gray-600">
      {options.map((o, i) => (
        <button
          key={o.key}
          type="button"
          aria-pressed={scope === o.key}
          onClick={() => onChange(o.key)}
          className={`px-3.5 py-2 text-sm font-medium focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500 ${
            i === 0 ? 'rounded-l-md' : 'rounded-r-md'
          } ${
            scope === o.key
              ? 'bg-indigo-600 text-white'
              : 'bg-white text-gray-700 hover:bg-gray-50 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700'
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function DurationCell({ job, now }: { job: JobSummary; now: number }) {
  const ms = elapsedMs(job, now);
  return (
    <span className={`font-mono tabular-nums ${job.status === 'running' ? 'text-blue-700 dark:text-blue-300' : ''}`}>
      {formatDuration(ms)}
    </span>
  );
}

export function DashboardPage() {
  const me = useMe();
  const isAdmin = Boolean(me.data?.is_admin);
  const [params, setParams] = useSearchParams();
  const status = parseJobStatus(params.get('status'));
  const scope: JobScope = isAdmin && params.get('scope') === 'all' ? 'all' : 'mine';
  const requestedBy = scope === 'all' ? (params.get('by') ?? '') : '';
  const search = params.get('q') ?? '';
  const sizeParam = Number.parseInt(params.get('size') ?? '', 10);
  const perPage = JOBS_PER_PAGE_OPTIONS.includes(sizeParam) ? sizeParam : JOBS_PER_PAGE;
  const page = Math.max(1, Number.parseInt(params.get('page') ?? '1', 10) || 1);

  const [byInput, setByInput] = useState(requestedBy);
  const debouncedBy = useDebouncedValue(byInput.trim().replace(/^@/, ''), 400);
  const [searchInput, setSearchInput] = useState(search);
  const debouncedSearch = useDebouncedValue(searchInput.trim(), 400);

  type View = { status?: JobStatus; scope: JobScope; by: string; q: string; size: number; page: number };
  /** Applies `changes` on top of the current view; any change other than the page returns to page 1. */
  const update = (changes: Partial<View>) => {
    const next: View = { status, scope, by: requestedBy, q: search, size: perPage, page: 1, ...changes };
    const p = new URLSearchParams();
    if (next.status) p.set('status', next.status);
    if (next.scope === 'all') p.set('scope', 'all');
    if (next.scope === 'all' && next.by) p.set('by', next.by);
    if (next.q) p.set('q', next.q);
    if (next.size !== JOBS_PER_PAGE) p.set('size', String(next.size));
    if (next.page > 1) p.set('page', String(next.page));
    setParams(p);
  };

  useEffect(() => {
    if (scope === 'all' && debouncedBy !== requestedBy) {
      update({ by: debouncedBy });
    }
    // Only the debounced input drives this; `update` is recreated every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [debouncedBy]);

  useEffect(() => {
    if (debouncedSearch !== search) {
      update({ q: debouncedSearch });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [debouncedSearch]);

  const jobs = useJobList({
    scope,
    status,
    requestedBy: requestedBy || undefined,
    search: search || undefined,
    page,
    perPage,
  });
  const items = jobs.data?.items ?? [];
  const now = useNow(items.some((j) => j.status === 'running'));
  const allUsers = scope === 'all';

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold">{allUsers ? 'All generations' : 'Your generations'}</h1>
          <p className="text-sm text-gray-600 dark:text-gray-300">
            {allUsers
              ? 'Documentation jobs of every user. Administrators can open pull requests on the requester\u2019s behalf.'
              : 'Bounded-context documentation jobs you started. Only you can see them.'}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          {isAdmin && (
            <ScopeSwitch
              scope={scope}
              onChange={(s) => {
                setByInput('');
                update({ scope: s, by: '' });
              }}
            />
          )}
          <Link to="/jobs/new" className={buttonClass('primary')}>
            New generation
          </Link>
        </div>
      </div>

      <StatusTiles
        selected={status}
        counts={jobs.data?.counts}
        onSelect={(s) => update({ status: s })}
      />

      <div className={cardClass}>
        <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-base font-semibold">
            {status ? TILES.find((t) => t.key === status)?.label : 'All'} jobs
            {jobs.data?.total !== undefined && (
              <span className="ml-2 text-sm font-normal text-gray-500">({countFormat.format(jobs.data.total)})</span>
            )}
          </h2>
          <div className="flex flex-wrap items-center gap-3">
            <div className="flex items-center gap-2">
              <label htmlFor="job-search" className="sr-only">
                Search jobs
              </label>
              <input
                id="job-search"
                type="search"
                value={searchInput}
                onChange={(e) => setSearchInput(e.target.value)}
                placeholder="Search repository, branch or job id"
                maxLength={100}
                autoComplete="off"
                spellCheck={false}
                className={`${inputClass} w-72`}
              />
            </div>
            {allUsers && (
              <div className="flex items-center gap-2">
                <label htmlFor="requested-by-filter" className="text-sm font-medium">
                  Requested by
                </label>
                <input
                  id="requested-by-filter"
                  type="search"
                  value={byInput}
                  onChange={(e) => setByInput(e.target.value)}
                  placeholder="GitHub username"
                  autoComplete="off"
                  spellCheck={false}
                  className={`${inputClass} w-52`}
                />
              </div>
            )}
            {jobs.isFetching && <Spinner size="sm" label="Refreshing" />}
            <ExportButton
              fileBase={allUsers ? 'all-generations' : 'my-generations'}
              disabled={!jobs.data || jobs.data.total === 0}
              build={async () => {
                const { items: all } = await listAllJobs({
                  status,
                  scope,
                  requested_by: allUsers ? requestedBy || undefined : undefined,
                  q: search || undefined,
                });
                return [{ name: allUsers ? 'All generations' : 'My generations', rows: jobRows(all) }];
              }}
            />
          </div>
        </div>

        {jobs.isPending && <Spinner label="Loading jobs…" />}
        {jobs.isError && <ErrorPanel error={jobs.error} title="Could not load jobs" />}
        {jobs.data && items.length === 0 && (
          <div className="py-10 text-center">
            <p className="text-sm text-gray-600 dark:text-gray-300">
              {status || requestedBy || search
                ? 'No jobs match this filter.'
                : allUsers
                  ? 'No user has generated documentation yet.'
                  : 'You have not generated any documentation yet.'}
            </p>
            {!status && !search && !allUsers && (
              <Link to="/jobs/new" className={`${linkClass} mt-2 inline-block`}>
                Start your first generation
              </Link>
            )}
          </div>
        )}
        {items.length > 0 && (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead className="border-b border-gray-200 text-xs uppercase text-gray-500 dark:border-gray-700 dark:text-gray-400">
                <tr>
                  <th scope="col" className="py-2 pr-4 font-medium">Repository</th>
                  {allUsers && <th scope="col" className="py-2 pr-4 font-medium">Requested by</th>}
                  <th scope="col" className="py-2 pr-4 font-medium">Branch</th>
                  <th scope="col" className="py-2 pr-4 font-medium">Status</th>
                  <th scope="col" className="py-2 pr-4 font-medium">Duration</th>
                  <th scope="col" className="py-2 pr-4 font-medium">Pull request</th>
                  <th scope="col" className="py-2 font-medium">Started</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100 dark:divide-gray-800">
                {items.map((job) => {
                  const pr = safeExternalUrl(job.pr_url);
                  return (
                    <tr key={job.id} className="hover:bg-gray-50 dark:hover:bg-gray-800/50">
                      <td className="py-3 pr-4">
                        <Link to={`/jobs/${encodeURIComponent(job.id)}`} className={linkClass}>
                          {job.owner}/{job.repo}
                        </Link>
                        <span className="ml-2 font-mono text-xs text-gray-500">{job.short_id}</span>
                        {job.batch_id && !allUsers && (
                          <Link
                            to={`/batches/${encodeURIComponent(job.batch_id)}`}
                            className={`${linkClass} ml-2 text-xs`}
                            aria-label={`Batch of ${job.owner}/${job.repo}`}
                          >
                            Batch
                          </Link>
                        )}
                      </td>
                      {allUsers && (
                        <td className="py-3 pr-4 text-xs">
                          <button
                            type="button"
                            className={linkClass}
                            title={`Show only jobs requested by ${job.requested_by}`}
                            onClick={() => {
                              setByInput(job.requested_by);
                              update({ by: job.requested_by });
                            }}
                          >
                            {job.requested_by}
                          </button>
                        </td>
                      )}
                      <td className="py-3 pr-4 font-mono text-xs">{job.base_branch}</td>
                      <td className="py-3 pr-4">
                        <StatusBadge status={job.status} />
                        {job.status === 'running' && (
                          <span className="ml-2 text-xs text-gray-500">{STAGE_LABELS[job.stage]}</span>
                        )}
                      </td>
                      <td className="py-3 pr-4 text-xs">
                        <DurationCell job={job} now={now} />
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
            total={jobs.data.total}
            perPage={perPage}
            perPageOptions={JOBS_PER_PAGE_OPTIONS}
            onPerPageChange={(size) => update({ size })}
            onPageChange={(p) => update({ page: p })}
            disabled={jobs.isFetching}
          />
        )}
      </div>
    </div>
  );
}
