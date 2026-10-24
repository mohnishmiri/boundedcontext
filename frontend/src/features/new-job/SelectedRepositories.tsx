import { useBranches, useRepository } from '../../api/queries';
import { Alert } from '../../components/Alert';
import { Button } from '../../components/Button';
import { ErrorPanel } from '../../components/ErrorPanel';
import { Spinner } from '../../components/Spinner';
import { inputClass } from '../../lib/styles';
import { MAX_BATCH_REPOS, repoKey, type SelectedRepo, type Selection } from './selection';

function SelectedRow({
  entry,
  branch,
  onBranchChange,
  onRemove,
}: {
  entry: SelectedRepo;
  /** Effective base branch (explicit choice, else the default branch). */
  branch: string | null;
  onBranchChange: (branch: string) => void;
  onRemove: () => void;
}) {
  const { owner, repo } = entry;
  const key = repoKey(owner, repo);
  const detail = useRepository(owner, repo);
  // Branches load lazily, once the repository is part of the selection.
  const branches = useBranches(owner, repo, '', 1);
  const names = branches.data?.items.map((b) => b.name) ?? [];
  const options = branch && !names.includes(branch) ? [branch, ...names] : names;
  const defaultBranch = entry.defaultBranch ?? detail.data?.default_branch ?? branches.data?.default_branch ?? null;
  const selectId = `branch-${key}`;

  return (
    <li className="space-y-2 px-3 py-3">
      <div className="flex flex-wrap items-center gap-3">
        <span className="min-w-0 flex-1 truncate font-medium">{key}</span>
        <div className="flex items-center gap-2">
          <span className="text-sm" aria-hidden="true">
            Branch
          </span>
          <select
            id={selectId}
            aria-label={`Base branch for ${key}`}
            className={`${inputClass} w-48 font-mono text-xs`}
            value={branch ?? ''}
            disabled={options.length === 0}
            onChange={(e) => onBranchChange(e.target.value)}
          >
            {options.length === 0 && <option value="">Loading…</option>}
            {options.map((name) => (
              <option key={name} value={name}>
                {name}
                {name === defaultBranch ? ' (default)' : ''}
              </option>
            ))}
          </select>
          <Button variant="ghost" aria-label={`Remove ${key}`} onClick={onRemove}>
            Remove
          </Button>
        </div>
      </div>
      {branches.isError && <ErrorPanel error={branches.error} title={`Could not load branches of ${key}`} />}
      {branches.data?.has_more && (
        <p className="text-xs text-gray-500 dark:text-gray-400">Showing the first {branches.data.items.length} branches.</p>
      )}
      <div aria-live="polite" className="space-y-2">
        {detail.isPending && <Spinner size="sm" label={`Checking access to ${key}…`} />}
        {detail.isError && <ErrorPanel error={detail.error} title={`Cannot use ${key}`} />}
        {detail.data && !detail.data.can_generate && (
          <Alert tone="error" title={`Documentation cannot be generated for ${key}`}>
            {detail.data.reason ?? 'You do not have the required access.'}
          </Alert>
        )}
        {detail.data && detail.data.can_generate && !detail.data.can_publish && (
          <Alert tone="warning" title={`You can generate for ${key}, but not open a pull request`}>
            {detail.data.reason ??
              'Opening a pull request requires push access to this repository. You can still review and download the result.'}
          </Alert>
        )}
      </div>
    </li>
  );
}

export function SelectedRepositories({
  selection,
  branchOf,
  onBranchChange,
  onRemove,
}: {
  selection: Selection;
  branchOf: (entry: SelectedRepo) => string | null;
  onBranchChange: (key: string, branch: string) => void;
  onRemove: (key: string) => void;
}) {
  const entries = Object.entries(selection);
  return (
    <fieldset className="space-y-3">
      <legend className="text-base font-semibold">
        Selected ({entries.length})
        <span className="ml-2 text-xs font-normal text-gray-500 dark:text-gray-400">max {MAX_BATCH_REPOS}</span>
      </legend>
      {entries.length === 0 ? (
        <p className="text-sm text-gray-600 dark:text-gray-300">
          No repositories selected yet. Each one gets its own job and its own base branch.
        </p>
      ) : (
        <ul
          aria-label="Selected repositories"
          className="divide-y divide-gray-100 rounded-md border border-gray-200 text-sm dark:divide-gray-800 dark:border-gray-700"
        >
          {entries.map(([key, entry]) => (
            <SelectedRow
              key={key}
              entry={entry}
              branch={branchOf(entry)}
              onBranchChange={(b) => onBranchChange(key, b)}
              onRemove={() => onRemove(key)}
            />
          ))}
        </ul>
      )}
    </fieldset>
  );
}
