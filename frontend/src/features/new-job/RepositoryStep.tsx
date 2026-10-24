import { useState } from 'react';

import { useRepositories } from '../../api/queries';
import type { Repository } from '../../api/types';
import { ErrorPanel } from '../../components/ErrorPanel';
import { Pagination } from '../../components/Pagination';
import { Spinner } from '../../components/Spinner';
import { useDebouncedValue } from '../../lib/hooks';
import { inputClass } from '../../lib/styles';
import { MAX_BATCH_REPOS, repoKey, type Selection } from './selection';

export function RepositoryStep({
  owner,
  selection,
  onToggle,
  error,
}: {
  owner: string;
  selection: Selection;
  onToggle: (repo: Repository, selected: boolean) => void;
  error?: string;
}) {
  const [query, setQuery] = useState('');
  const [page, setPage] = useState(1);
  const q = useDebouncedValue(query.trim(), 300);
  const repos = useRepositories(owner, q, page);
  const full = Object.keys(selection).length >= MAX_BATCH_REPOS;

  return (
    <fieldset className="space-y-3">
      <legend className="text-base font-semibold">3. Repositories</legend>
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
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Tick every repository to document. Your selection is kept when you search, change page or switch account.
      </p>
      {full && (
        <p className="text-sm text-amber-800 dark:text-amber-200" role="status">
          You can select up to {MAX_BATCH_REPOS} repositories per batch. Remove one to select another.
        </p>
      )}
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
            const checked = repoKey(r.owner, r.name) in selection;
            const blocked = full && !checked;
            return (
              <li key={r.id}>
                <label
                  className={`flex items-start gap-3 px-3 py-2 text-sm focus-within:bg-indigo-50 dark:focus-within:bg-indigo-950/40 ${
                    blocked ? 'cursor-not-allowed opacity-60' : 'cursor-pointer'
                  } ${checked ? 'bg-indigo-50 dark:bg-indigo-950/40' : blocked ? '' : 'hover:bg-gray-50 dark:hover:bg-gray-800'}`}
                >
                  <input
                    type="checkbox"
                    name="repo"
                    value={r.name}
                    checked={checked}
                    disabled={blocked}
                    onChange={(e) => onToggle(r, e.target.checked)}
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
                      {!r.permissions.pull && (
                        <span className="rounded bg-red-100 px-1.5 text-xs text-red-800 dark:bg-red-950 dark:text-red-200">
                          no read access
                        </span>
                      )}
                      {r.permissions.pull && !r.permissions.push && (
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
    </fieldset>
  );
}
