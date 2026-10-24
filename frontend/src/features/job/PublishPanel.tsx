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
  return `/jobs/new?${p.toString()}`;
}

export function PublishPanel({ job }: { job: Job }) {
  const qc = useQueryClient();
  const publish = useMutation({
    mutationFn: () => api.requestPullRequest(job.id),
    onSuccess: (next) => qc.setQueryData(queryKeys.job(job.id), next),
  });
  const p = job.publish;
  const prUrl = safeExternalUrl(p.pr_url);
  const commitUrl = safeExternalUrl(p.commit_url);
  const brUrl = p.branch ? branchUrl(job.repository_url, p.branch) : undefined;

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
            <Button onClick={() => publish.mutate()} loading={publish.isPending}>
              Create pull request
            </Button>
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
              <Button variant="secondary" onClick={() => publish.mutate()} loading={publish.isPending}>
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
