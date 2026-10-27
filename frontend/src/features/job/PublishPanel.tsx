import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Link } from 'react-router-dom';

import * as api from '../../api/client';
import { queryKeys } from '../../api/queries';
import type { Job } from '../../api/types';
import { Alert } from '../../components/Alert';
import { Button } from '../../components/Button';
import { ErrorPanel } from '../../components/ErrorPanel';
import { StepList } from '../../components/StepList';
import { branchUrl, safeExternalUrl, shortSha } from '../../lib/format';
import { linkClass } from '../../lib/styles';

function regenerateLink(job: Job): string {
  const p = new URLSearchParams({ owner: job.owner, repo: job.repo, branch: job.base_branch, model: job.model });
  if (job.generator?.id) p.set('generator', job.generator.id);
  return `/jobs/new?${p.toString()}`;
}

export function PublishPanel({ job }: { job: Job }) {
  const qc = useQueryClient();
  const p = job.publish;
  const [title, setTitle] = useState(p.title ?? '');
  const [publishCentral, setPublishCentral] = useState(p.central_requested);
  const publish = useMutation({
    mutationFn: (body: { title: string; publish_central: boolean }) => api.requestPullRequest(job.id, body),
    onSuccess: (next) => qc.setQueryData(queryKeys.job(job.id), next),
  });
  const prUrl = safeExternalUrl(p.pr_url);
  const centralPrUrl = safeExternalUrl(p.central_pr_url);
  const commitUrl = safeExternalUrl(p.commit_url);
  const brUrl = p.branch ? branchUrl(job.repository_url, p.branch) : undefined;
  const submitPublish = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    publish.mutate({ title: title.trim(), publish_central: publishCentral });
  };

  return (
    <section aria-labelledby="publish-heading" className="space-y-4">
      <h2 id="publish-heading" className="text-lg font-semibold">
        Pull request
      </h2>

      {p.status === 'not_started' &&
        (job.can_publish ? (
          <div className="space-y-3">
            <p className="text-sm text-gray-600 dark:text-gray-300">
              Creates a new branch from <code className="font-mono">{shortSha(job.base_sha)}</code> with a single commit
              containing only the generated files, and opens a pull request against{' '}
              <code className="font-mono">{job.base_branch}</code>. It is never merged automatically.
            </p>
            <form onSubmit={submitPublish} className="space-y-3">
              <label className="block space-y-1 text-sm font-medium" htmlFor="publish-title">
                <span>Pull request title</span>
                <input
                  id="publish-title"
                  name="title"
                  required
                  maxLength={256}
                  value={title}
                  onChange={(event) => setTitle(event.target.value)}
                  className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm font-normal text-gray-900 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/30 dark:border-gray-700 dark:bg-gray-900 dark:text-gray-100"
                  placeholder="APM-123: Update bounded context documentation"
                />
              </label>
              {p.central_available && (
                <label className="flex items-start gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={publishCentral}
                    onChange={(event) => setPublishCentral(event.target.checked)}
                    className="mt-1 h-4 w-4 accent-indigo-600"
                  />
                  <span>
                    Also create a pull request in <code className="font-mono">{p.central_repo}</code>
                  </span>
                </label>
              )}
              <Button type="submit" disabled={!title.trim()} loading={publish.isPending}>
                Create pull request
              </Button>
            </form>
          </div>
        ) : (
          <Alert tone="info">
            You cannot open a pull request for this repository — push access is required. You can still download the
            documentation and share it with a maintainer.
          </Alert>
        ))}

      {p.status !== 'not_started' && <StepList steps={p.steps} label="Publishing steps" />}

      {(p.status === 'queued' || p.status === 'running') && (
        <p className="text-sm text-gray-600 dark:text-gray-300">Publishing to GitHub…</p>
      )}

      {p.status === 'succeeded' && (
        <Alert tone="success" title={p.pr_number ? `Pull request #${p.pr_number} is open` : 'Pull request is open'}>
          <ul className="space-y-1">
            {prUrl && (
              <li>
                <a href={prUrl} target="_blank" rel="noopener noreferrer" className={linkClass}>
                  Open pull request{p.pr_number ? ` #${p.pr_number}` : ''}
                </a>
              </li>
            )}
            {centralPrUrl && (
              <li>
                <a href={centralPrUrl} target="_blank" rel="noopener noreferrer" className={linkClass}>
                  Open central repository pull request{p.central_pr_number ? ` #${p.central_pr_number}` : ''}
                </a>
              </li>
            )}
            {p.branch && (
              <li>
                Branch:{' '}
                {brUrl ? (
                  <a href={brUrl} target="_blank" rel="noopener noreferrer" className={`${linkClass} font-mono`}>
                    {p.branch}
                  </a>
                ) : (
                  <code className="font-mono">{p.branch}</code>
                )}
              </li>
            )}
            {p.commit_sha && (
              <li>
                Commit:{' '}
                {commitUrl ? (
                  <a href={commitUrl} target="_blank" rel="noopener noreferrer" className={`${linkClass} font-mono`}>
                    {shortSha(p.commit_sha)}
                  </a>
                ) : (
                  <code className="font-mono">{shortSha(p.commit_sha)}</code>
                )}
              </li>
            )}
          </ul>
          <p>Review and merge it on GitHub. The portal never merges pull requests.</p>
        </Alert>
      )}

      {p.status === 'failed' && p.error && (
        <ErrorPanel error={p.error} title="Publishing failed">
          {p.error.code === 'base_branch_changed' ? (
            <Link to={regenerateLink(job)} className={linkClass}>
              Generate again on the latest commit
            </Link>
          ) : (
            job.can_publish && (
              <Button
                variant="secondary"
                disabled={!title.trim()}
                onClick={() => publish.mutate({ title: title.trim(), publish_central: publishCentral })}
                loading={publish.isPending}
              >
                Try publishing again
              </Button>
            )
          )}
        </ErrorPanel>
      )}

      {publish.isError && <ErrorPanel error={publish.error} title="Could not start publishing" />}
    </section>
  );
}
