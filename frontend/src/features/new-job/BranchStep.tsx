import { useState } from 'react';

import { useBranches } from '../../api/queries';
import { ErrorPanel } from '../../components/ErrorPanel';
import { Pagination } from '../../components/Pagination';
import { Spinner } from '../../components/Spinner';
import { useDebouncedValue } from '../../lib/hooks';
import { inputClass } from '../../lib/styles';

export function BranchStep({
  owner,
  repo,
  branch,
  defaultBranch,
  onChange,
  error,
}: {
  owner: string;
  repo: string;
  branch: string | null;
  defaultBranch: string | null;
  onChange: (branch: string) => void;
  error?: string;
}) {
  const [query, setQuery] = useState('');
  const [page, setPage] = useState(1);
  const q = useDebouncedValue(query.trim(), 300);
  const branches = useBranches(owner, repo, q, page);
  const def = defaultBranch ?? branches.data?.default_branch ?? null;

  return (
    <fieldset className="space-y-3">
      <legend className="text-base font-semibold">3. Base branch</legend>
      <p className="text-sm" aria-live="polite">
        {branch ? (
          <>
            Analysing <code className="rounded bg-gray-100 px-1 py-0.5 dark:bg-gray-800">{branch}</code>
            {branch === def && <span className="text-gray-500"> (default branch)</span>}
          </>
        ) : (
          <span className="text-gray-500">Choose a branch.</span>
        )}
      </p>
      <div>
        <label htmlFor="branch-search" className="mb-1 block text-sm font-medium">
          Search branches
        </label>
        <input
          id="branch-search"
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
      {branches.isPending && <Spinner label="Loading branches…" />}
      {branches.isError && <ErrorPanel error={branches.error} title="Could not load branches" />}
      {branches.data && branches.data.items.length === 0 && (
        <p className="text-sm text-gray-600 dark:text-gray-300">No branches match.</p>
      )}
      {branches.data && branches.data.items.length > 0 && (
        <ul className="max-h-64 divide-y divide-gray-100 overflow-y-auto rounded-md border border-gray-200 dark:divide-gray-800 dark:border-gray-700">
          {branches.data.items.map((b) => {
            const checked = branch === b.name;
            return (
              <li key={b.name}>
                <label
                  className={`flex cursor-pointer items-center gap-3 px-3 py-2 text-sm focus-within:bg-indigo-50 dark:focus-within:bg-indigo-950/40 ${
                    checked ? 'bg-indigo-50 dark:bg-indigo-950/40' : 'hover:bg-gray-50 dark:hover:bg-gray-800'
                  }`}
                >
                  <input
                    type="radio"
                    name="branch"
                    value={b.name}
                    checked={checked}
                    onChange={() => onChange(b.name)}
                    className="h-4 w-4 accent-indigo-600"
                  />
                  <span className="font-mono text-xs">{b.name}</span>
                  {b.name === def && (
                    <span className="rounded bg-indigo-100 px-1.5 text-xs text-indigo-800 dark:bg-indigo-950 dark:text-indigo-200">
                      default
                    </span>
                  )}
                  {b.protected && (
                    <span className="rounded bg-gray-100 px-1.5 text-xs text-gray-600 dark:bg-gray-800 dark:text-gray-300">
                      protected
                    </span>
                  )}
                  <span className="ml-auto font-mono text-xs text-gray-400">{b.commit_sha.slice(0, 7)}</span>
                </label>
              </li>
            );
          })}
        </ul>
      )}
      {branches.data && (
        <Pagination
          label="Branch pages"
          page={page}
          hasMore={branches.data.has_more}
          onPageChange={setPage}
          disabled={branches.isFetching}
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
