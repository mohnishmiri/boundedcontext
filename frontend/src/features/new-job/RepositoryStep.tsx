import { useState } from 'react';

import { useRepositories, useRepository } from '../../api/queries';
import { Alert } from '../../components/Alert';
import { ErrorPanel } from '../../components/ErrorPanel';
import { Pagination } from '../../components/Pagination';
import { Spinner } from '../../components/Spinner';
import { useDebouncedValue } from '../../lib/hooks';
import { inputClass } from '../../lib/styles';

export function RepositoryStep({
  owner,
  repo,
  onChange,
  error,
}: {
  owner: string;
  repo: string | null;
  onChange: (repo: string) => void;
  error?: string;
}) {
  const [query, setQuery] = useState('');
  const [page, setPage] = useState(1);
  const q = useDebouncedValue(query.trim(), 300);
  const repos = useRepositories(owner, q, page);
  const detail = useRepository(owner, repo);

  return (
    <fieldset className="space-y-3">
      <legend className="text-base font-semibold">2. Repository</legend>
      <div>
        <label htmlFor="repo-search" className="mb-1 block text-sm font-medium">
          Search repositories in {owner}
        </label>
        <input
          id="repo-search"
          type="search"
          className={inputClass}
          placeholder="Filter by name"
          value={query}
          autoComplete="off"
          onChange={(e) => {
            setQuery(e.target.value);
            setPage(1);
          }}
        />
      </div>
      {repos.isPending && <Spinner label="Loading repositories…" />}
      {repos.isError && <ErrorPanel error={repos.error} title="Could not load repositories" />}
      {repos.data && repos.data.items.length === 0 && (
        <p className="text-sm text-gray-600 dark:text-gray-300">
          No repositories found. You only see repositories you can access where the GitHub App is installed.
        </p>
      )}
      {repos.data && repos.data.items.length > 0 && (
        <ul className="max-h-80 divide-y divide-gray-100 overflow-y-auto rounded-md border border-gray-200 dark:divide-gray-800 dark:border-gray-700">
          {repos.data.items.map((r) => {
            const checked = repo === r.name;
            return (
              <li key={r.id}>
                <label
                  className={`flex cursor-pointer items-start gap-3 px-3 py-2 text-sm focus-within:bg-indigo-50 dark:focus-within:bg-indigo-950/40 ${
                    checked ? 'bg-indigo-50 dark:bg-indigo-950/40' : 'hover:bg-gray-50 dark:hover:bg-gray-800'
                  }`}
                >
                  <input
                    type="radio"
                    name="repo"
                    value={r.name}
                    checked={checked}
                    onChange={() => onChange(r.name)}
                    className="mt-1 h-4 w-4 accent-indigo-600"
                  />
                  <span className="min-w-0 flex-1">
                    <span className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{r.full_name}</span>
                      {r.private && (
                        <span className="rounded bg-gray-100 px-1.5 text-xs text-gray-600 dark:bg-gray-800 dark:text-gray-300">
                          private
                        </span>
                      )}
                      {!r.permissions.push && (
                        <span className="rounded bg-amber-100 px-1.5 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-200">
                          read only
                        </span>
                      )}
                    </span>
                    {r.description && (
                      <span className="block truncate text-xs text-gray-500 dark:text-gray-400">{r.description}</span>
                    )}
                  </span>
                </label>
              </li>
            );
          })}
        </ul>
      )}
      {repos.data && (
        <Pagination
          label="Repository pages"
          page={page}
          hasMore={repos.data.has_more}
          onPageChange={setPage}
          disabled={repos.isFetching}
        />
      )}
      {error && (
        <p className="text-sm text-red-600 dark:text-red-400" role="alert">
          {error}
        </p>
      )}
      {repo && (
        <div aria-live="polite" className="space-y-2">
          {detail.isPending && <Spinner size="sm" label={`Checking access to ${owner}/${repo}…`} />}
          {detail.isError && <ErrorPanel error={detail.error} title={`Cannot use ${owner}/${repo}`} />}
          {detail.data && !detail.data.can_generate && (
            <Alert tone="error" title="Documentation cannot be generated for this repository">
              {detail.data.reason ?? 'You do not have the required access.'}
            </Alert>
          )}
          {detail.data && detail.data.can_generate && !detail.data.can_publish && (
            <Alert tone="warning" title="You can generate, but not open a pull request">
              {detail.data.reason ??
                'Opening a pull request requires push access to this repository. You can still review and download the result.'}
            </Alert>
          )}
          {detail.data && detail.data.can_generate && detail.data.can_publish && (
            <p className="text-sm text-gray-600 dark:text-gray-300">
              Selected <strong>{detail.data.full_name}</strong>. You can generate documentation and open a pull request.
            </p>
          )}
        </div>
      )}
    </fieldset>
  );
}
