import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';

import type { Job } from '../../api/types';
import { PublishBadge, StatusBadge } from '../../components/StatusBadge';
import { formatDateTime, formatDuration, safeExternalUrl, shortSha } from '../../lib/format';
import { useNow } from '../../lib/hooks';
import { elapsedMs } from '../../lib/jobState';
import { linkClass } from '../../lib/styles';

const count = new Intl.NumberFormat();
const creditFormat = new Intl.NumberFormat(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });

function Detail({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">{label}</dt>
      <dd className="mt-1 truncate text-sm text-gray-900 dark:text-gray-100">{children}</dd>
    </div>
  );
}

function Stat({ label, value, hint, accent }: { label: string; value: ReactNode; hint?: ReactNode; accent?: string }) {
  return (
    <div className="min-w-0 rounded-lg bg-gray-50 px-4 py-3 ring-1 ring-inset ring-gray-200 dark:bg-gray-800/60 dark:ring-gray-700">
      <dt className="text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">{label}</dt>
      <dd className={`mt-1 text-xl font-semibold tabular-nums ${accent ?? 'text-gray-900 dark:text-white'}`}>{value}</dd>
      {hint && <dd className="mt-0.5 truncate text-xs text-gray-500 dark:text-gray-400">{hint}</dd>}
    </div>
  );
}

function RepoIcon() {
  return (
    <svg viewBox="0 0 16 16" className="h-5 w-5 shrink-0 text-gray-400" fill="currentColor" aria-hidden="true">
      <path d="M2 2.5A2.5 2.5 0 0 1 4.5 0h8.75a.75.75 0 0 1 .75.75v12.5a.75.75 0 0 1-.75.75h-2.5a.75.75 0 0 1 0-1.5h1.75v-2h-8a1 1 0 0 0-.714 1.7.75.75 0 1 1-1.072 1.05A2.495 2.495 0 0 1 2 11.5Zm10.5-1h-8a1 1 0 0 0-1 1v6.708A2.486 2.486 0 0 1 4.5 9h8ZM5 12.25a.25.25 0 0 1 .25-.25h3.5a.25.25 0 0 1 .25.25v3.25a.25.25 0 0 1-.4.2l-1.45-1.087a.249.249 0 0 0-.3 0L5.4 15.7a.25.25 0 0 1-.4-.2Z" />
    </svg>
  );
}

/** Job summary: identity and status, key figures, then run details. `actions` render top right. */
export function JobHeader({ job, live, actions }: { job: Job; live: boolean; actions?: ReactNode }) {
  const running = live && job.status === 'running';
  const now = useNow(running);
  const repoUrl = safeExternalUrl(job.repository_url);
  // Empty optional inputs were not used by the run; don't show them.
  const inputs = Object.entries(job.inputs).filter(([, v]) => v !== '');
  // The server's duration_ms is a snapshot; while running, count from started_at so the clock ticks every second.
  const duration = running ? elapsedMs(job, now) : (job.duration_ms ?? elapsedMs(job, now));
  const usage = job.usage;
  const repoName = (
    <>
      <span className="font-normal text-gray-500 dark:text-gray-400">{job.owner} / </span>
      {job.repo}
    </>
  );

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0 space-y-2">
          <div className="flex flex-wrap items-center gap-3">
            <RepoIcon />
            <h1 className="min-w-0 break-words text-2xl font-semibold tracking-tight">
              {repoUrl ? (
                <a href={repoUrl} target="_blank" rel="noopener noreferrer" className="hover:underline">
                  {repoName}
                </a>
              ) : (
                repoName
              )}
            </h1>
            <StatusBadge status={job.status} />
            {job.status === 'succeeded' && <PublishBadge status={job.publish.status} />}
          </div>
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-gray-600 dark:text-gray-300">
            {job.generator && (
              <>
                <span
                  className="font-medium text-gray-900 dark:text-gray-100"
                  title={job.generator.owner && job.generator.repo ? `${job.generator.owner}/${job.generator.repo}` : undefined}
                >
                  {job.generator.name}
                </span>
                {job.generator.source !== 'builtin' && (
                  <span className="text-xs text-gray-500 dark:text-gray-400">({job.generator.source})</span>
                )}
                <span aria-hidden="true">·</span>
              </>
            )}
            <span>
              branch <code className="rounded bg-gray-100 px-1.5 py-0.5 font-mono text-xs dark:bg-gray-800">{job.base_branch}</code>
            </span>
            <span>
              at{' '}
              <code className="font-mono text-xs" title={job.base_sha ?? undefined}>
                {shortSha(job.base_sha)}
              </code>
            </span>
          </p>
        </div>
        {actions && <div className="flex shrink-0 flex-wrap items-start gap-2">{actions}</div>}
      </div>

      <dl className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Stat
          label={running ? 'Elapsed' : 'Duration'}
          value={<span className="font-mono">{formatDuration(duration)}</span>}
          accent={running ? 'text-blue-700 dark:text-blue-300' : undefined}
          hint={
            running ? (
              <span className="inline-flex items-center gap-1 text-blue-700 dark:text-blue-300">
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-current" aria-hidden="true" />
                live
              </span>
            ) : job.finished_at ? (
              `Finished ${formatDateTime(job.finished_at)}`
            ) : undefined
          }
        />
        <Stat
          label="AI credits (est.)"
          value={usage ? creditFormat.format(usage.ai_credits ?? 0) : '—'}
          hint={usage && usage.runs > 1 ? `${usage.runs} runs` : usage?.shared_token ? 'Shared Copilot account' : undefined}
        />
        <Stat label="Premium requests" value={usage ? count.format(usage.premium_requests) : '—'} />
        <Stat label="Copilot API time" value={usage ? formatDuration(usage.api_duration_ms) : '—'} />
      </dl>
      {usage && usage.models.length > 0 && (
        <ul aria-label="Usage by model" className="flex flex-wrap gap-2 text-xs">
          {usage.models.map((m) => (
            <li
              key={m.model}
              className="rounded-full bg-gray-100 px-2.5 py-1 text-gray-700 dark:bg-gray-800 dark:text-gray-300"
            >
              <span className="font-mono">{m.model}</span> · {creditFormat.format(m.ai_credits ?? 0)} credits ·{' '}
              {count.format(m.premium_requests)} premium
            </li>
          ))}
        </ul>
      )}
      {usage?.shared_token && (
        <p className="text-xs text-amber-700 dark:text-amber-300">
          Billed to the portal&apos;s shared Copilot account, not the requester&apos;s own budget.
        </p>
      )}

      <dl className="grid grid-cols-2 gap-x-6 gap-y-4 border-t border-gray-200 pt-5 sm:grid-cols-3 lg:grid-cols-6 dark:border-gray-700">
        <Detail label="Requested by">{job.requested_by}</Detail>
        <Detail label="Created">{formatDateTime(job.created_at)}</Detail>
        <Detail label="Model">{job.model}</Detail>
        <Detail label="Agent">
          <span className="font-mono text-xs" title={`${job.agent.id} ${job.agent.version}`}>
            {job.agent.id} {job.agent.version}
          </span>
          {job.agent.placeholder && <span className="text-amber-700 dark:text-amber-300"> (placeholder)</span>}
        </Detail>
        <Detail label="Job">
          <span className="font-mono text-xs">{job.short_id}</span>
          {job.batch_id && (
            <>
              {' · '}
              <Link to={`/batches/${encodeURIComponent(job.batch_id)}`} className={linkClass}>
                Batch
              </Link>
            </>
          )}
        </Detail>
        <Detail label="Inputs">
          {inputs.length > 0 ? (
            inputs.map(([k, v], i) => (
              <span key={k} title={`${k}=${v}`}>
                {i > 0 && ', '}
                {k}=<span className="font-mono">{v}</span>
              </span>
            ))
          ) : (
            <span className="text-gray-400">—</span>
          )}
        </Detail>
      </dl>
    </div>
  );
}
