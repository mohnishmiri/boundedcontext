import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';

import type { Job } from '../../api/types';
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
