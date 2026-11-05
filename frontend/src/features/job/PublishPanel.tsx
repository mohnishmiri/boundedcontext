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
import { inputClass, linkClass } from '../../lib/styles';

function regenerateLink(job: Job): string {
  const p = new URLSearchParams({ owner: job.owner, repo: job.repo, branch: job.base_branch, model: job.model });
  if (job.generator?.id) p.set('generator', job.generator.id);
  return `/jobs/new?${p.toString()}`;
}

/** "alice, @bob carol" -> ["alice", "bob", "carol"] */
function parseReviewers(text: string): string[] {
  return text
    .split(/[\s,;]+/)
    .map((r) => r.replace(/^@/, ''))
    .filter(Boolean);
}

const REPO_SLUG = /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})\/[A-Za-z0-9._-]{1,100}$/;

export function PublishPanel({ job, readOnly = false }: { job: Job; readOnly?: boolean }) {
  const qc = useQueryClient();
  const p = job.publish;
  const defaultCentral = p.central_repo ?? '';
  const [title, setTitle] = useState(p.title ?? '');
  const [reviewers, setReviewers] = useState((p.reviewers ?? []).join(', '));
  const [publishCentral, setPublishCentral] = useState(p.central_requested);
  const [centralRepo, setCentralRepo] = useState(defaultCentral);
  const centralTrimmed = centralRepo.trim();
  const centralInvalid = publishCentral && !REPO_SLUG.test(centralTrimmed);
  const publish = useMutation({
    mutationFn: (body: api.PullRequestBody) => api.requestPullRequest(job.id, body),
    onSuccess: (next) => qc.setQueryData(queryKeys.job(job.id), next),
  });
  const body = (): api.PullRequestBody => ({
    title: title.trim(),
    publish_central: publishCentral,
    reviewers: parseReviewers(reviewers),
    ...(publishCentral && centralTrimmed !== defaultCentral ? { central_repo: centralTrimmed } : {}),
  });
  const canSubmit = Boolean(title.trim()) && !centralInvalid;
  const prUrl = safeExternalUrl(p.pr_url);
  const centralPrUrl = safeExternalUrl(p.central_pr_url);
  const commitUrl = safeExternalUrl(p.commit_url);
  const brUrl = p.branch ? branchUrl(job.repository_url, p.branch) : undefined;
  const submitPublish = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    publish.mutate(body());
  };

  return (
    <section aria-labelledby="publish-heading" className="space-y-4">
      <h2 id="publish-heading" className="text-lg font-semibold">
        Pull request
      </h2>

      {p.status === 'not_started' &&
        (job.can_publish ? (
          <div className="space-y-3">
            {readOnly && (
              <Alert tone="warning" title="Publishing on behalf of the requester">
                You are opening this pull request for <strong>{job.requested_by}</strong> with your own GitHub access.
                You need push access to the repository.
              </Alert>
            )}
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
              <label className="block space-y-1 text-sm font-medium" htmlFor="publish-reviewers">
                <span>Reviewers (optional)</span>
                <input
                  id="publish-reviewers"
                  name="reviewers"
                  value={reviewers}
                  onChange={(event) => setReviewers(event.target.value)}
                  autoComplete="off"
                  className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm font-normal text-gray-900 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/30 dark:border-gray-700 dark:bg-gray-900 dark:text-gray-100"
                  placeholder="GitHub usernames, comma-separated (e.g. alice_ATT, bob_ATT)"
                />
                <span className="block text-xs font-normal text-gray-500 dark:text-gray-400">
                  Requested on every pull request opened. Reviewers must have access to the repository.
                </span>
              </label>
              <fieldset className="space-y-2 rounded-md border border-gray-200 p-3 dark:border-gray-700">
                <legend className="sr-only">Central repository</legend>
                <label className="flex items-start gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={publishCentral}
                    onChange={(event) => setPublishCentral(event.target.checked)}
                    className="mt-1 h-4 w-4 accent-indigo-600"
                  />
                  <span>
                    Also create a pull request in a central repository
                    {defaultCentral && (
                      <span className="block text-xs text-gray-500 dark:text-gray-400">
                        Default for this generator: <code className="font-mono">{defaultCentral}</code>
                      </span>
                    )}
                  </span>
                </label>
                {publishCentral && (
                  <div className="space-y-1 pl-6">
                    <label className="block text-sm font-medium" htmlFor="publish-central-repo">
                      Central repository
                    </label>
                    <div className="flex flex-wrap items-center gap-2">
                      <input
                        id="publish-central-repo"
                        name="central_repo"
                        value={centralRepo}
                        onChange={(event) => setCentralRepo(event.target.value)}
                        autoComplete="off"
                        spellCheck={false}
                        aria-invalid={centralInvalid}
                        aria-describedby="publish-central-repo-help"
                        className={`${inputClass} max-w-md font-mono`}
                        placeholder="owner/repository"
                      />
                      {defaultCentral && centralTrimmed !== defaultCentral && (
                        <Button variant="ghost" onClick={() => setCentralRepo(defaultCentral)}>
                          Reset to default
                        </Button>
                      )}
                    </div>
                    <p id="publish-central-repo-help" className="text-xs text-gray-500 dark:text-gray-400">
                      {centralInvalid
                        ? 'Enter the repository as owner/repository.'
                        : 'Any repository you can push to. The files are added under docs/<repository>/.'}
                    </p>
                  </div>
                )}
              </fieldset>
              <Button type="submit" disabled={!canSubmit} loading={publish.isPending}>
                {readOnly ? `Create pull request for ${job.requested_by}` : 'Create pull request'}
              </Button>
            </form>
          </div>
        ) : readOnly ? (
          <p className="text-sm text-gray-600 dark:text-gray-300">No pull request has been opened for this job yet.</p>
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
                disabled={!canSubmit}
                onClick={() => publish.mutate(body())}
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
