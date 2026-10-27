import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';

import type { Job, JobUsage } from '../../api/types';
import { PublishBadge, StatusBadge } from '../../components/StatusBadge';
import { formatDateTime, formatDuration, safeExternalUrl, shortSha } from '../../lib/format';
import { useNow } from '../../lib/hooks';
import { linkClass } from '../../lib/styles';

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <dt className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">{label}</dt>
      <dd className="mt-0.5 text-sm">{children}</dd>
    </div>
  );
}

const count = new Intl.NumberFormat();

function UsageDetails({ usage }: { usage: JobUsage }) {
  return (
    <section aria-label="Copilot usage" className="rounded-md border border-gray-200 p-3 dark:border-gray-700">
      <h2 className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
        Copilot usage{usage.runs > 1 ? ` (${usage.runs} runs)` : ''}
      </h2>
      <dl className="mt-2 grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-6">
        {usage.ai_credits !== undefined && usage.ai_credits > 0 && (
          <Field label="AI credits (est.)">{usage.ai_credits.toFixed(2)}</Field>
        )}
        <Field label="Premium requests">{count.format(usage.premium_requests)}</Field>
        <Field label="Input tokens">{count.format(usage.input_tokens)}</Field>
        <Field label="Output tokens">{count.format(usage.output_tokens)}</Field>
        <Field label="Cache read / write">
          {count.format(usage.cache_read_tokens)} / {count.format(usage.cache_write_tokens)}
        </Field>
        <Field label="API time">{formatDuration(usage.api_duration_ms)}</Field>
      </dl>
      {usage.models.length > 0 && (
        <ul className="mt-2 space-y-0.5 text-xs text-gray-600 dark:text-gray-300">
          {usage.models.map((m) => (
            <li key={m.model}>
              <span className="font-mono">{m.model}</span>: {count.format(m.requests)} requests,{' '}
              {count.format(m.input_tokens)} in / {count.format(m.output_tokens)} out
              {m.reasoning_tokens > 0 && `, ${count.format(m.reasoning_tokens)} reasoning`}
            </li>
          ))}
        </ul>
      )}
      {usage.shared_token && (
        <p className="mt-2 text-xs text-amber-700 dark:text-amber-300">
          Billed to the portal&apos;s shared Copilot account, not the requester&apos;s own budget.
        </p>
      )}
    </section>
  );
}

export function JobHeader({ job, live }: { job: Job; live: boolean }) {
  const now = useNow(live && job.status === 'running');
  const repoUrl = safeExternalUrl(job.repository_url);
  // Empty optional inputs were not used by the run; don't show them.
  const inputs = Object.entries(job.inputs).filter(([, v]) => v !== '');
  const duration =
    job.duration_ms ?? (job.started_at && job.status === 'running' ? now - new Date(job.started_at).getTime() : null);

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-2xl font-semibold">
          {repoUrl ? (
            <a href={repoUrl} target="_blank" rel="noopener noreferrer" className="hover:underline">
              {job.owner}/{job.repo}
            </a>
          ) : (
            `${job.owner}/${job.repo}`
          )}
        </h1>
        <StatusBadge status={job.status} />
        {job.status === 'succeeded' && <PublishBadge status={job.publish.status} />}
      </div>
      <dl className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
        <Field label="Branch">
          <code className="font-mono text-xs">{job.base_branch}</code>
        </Field>
        <Field label="Commit">
          <code className="font-mono text-xs" title={job.base_sha ?? undefined}>
            {shortSha(job.base_sha)}
          </code>
        </Field>
        {job.generator && (
          <Field label="Generator">
            <span title={job.generator.owner && job.generator.repo ? `${job.generator.owner}/${job.generator.repo}` : undefined}>
              {job.generator.name}
            </span>
            {job.generator.source !== 'builtin' && (
              <span className="ml-1 text-xs text-gray-500 dark:text-gray-400">({job.generator.source})</span>
            )}
          </Field>
        )}
        <Field label="Model">{job.model}</Field>
        <Field label="Requested by">{job.requested_by}</Field>
        <Field label="Created">{formatDateTime(job.created_at)}</Field>
        <Field label={job.status === 'running' ? 'Elapsed' : 'Duration'}>{formatDuration(duration)}</Field>
      </dl>
      {job.usage && <UsageDetails usage={job.usage} />}
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Job <span className="font-mono">{job.short_id}</span>
        {' · agent '}<span className="font-mono">{job.agent.id}</span>{' '}
        <span className="font-mono">{job.agent.version}</span>
        {job.agent.placeholder && <span className="text-amber-700 dark:text-amber-300"> (placeholder)</span>}
        {inputs.length > 0 && (
          <>
            {' · '}
            {inputs.map(([k, v], i) => (
              <span key={k}>
                {i > 0 && ', '}
                {k}=<span className="font-mono">{v}</span>
              </span>
            ))}
          </>
        )}
        {job.batch_id && (
          <>
            {' · '}
            <Link to={`/batches/${encodeURIComponent(job.batch_id)}`} className={linkClass}>
              Batch
            </Link>
          </>
        )}
        {repoUrl && (
          <>
            {' · '}
            <a href={repoUrl} target="_blank" rel="noopener noreferrer" className={linkClass}>
              Repository
            </a>
          </>
        )}
      </p>
    </div>
  );
}
