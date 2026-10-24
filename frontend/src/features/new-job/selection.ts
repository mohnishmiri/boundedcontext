/** Repositories picked for a batch, keyed by `owner/repo` so the selection survives organisation switches. */
export type SelectedRepo = {
  owner: string;
  repo: string;
  /** Known when picked from the repository list; filled in from the repository details otherwise. */
  defaultBranch: string | null;
  /** Explicit choice; `null` means "use the default branch". */
  branch: string | null;
};

export type Selection = Record<string, SelectedRepo>;

/** Server-side limit of repositories in one batch. */
export const MAX_BATCH_REPOS = 25;

export function repoKey(owner: string, repo: string): string {
  return `${owner}/${repo}`;
}
