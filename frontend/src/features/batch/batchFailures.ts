import type { BatchFailure } from '../../api/types';

// Failures are only returned by the POST that created the batch. They travel to the batch page through router
// state and are mirrored in sessionStorage (keyed by batch id) so that a reload keeps them.

const storageKey = (batchId: string) => `bcp:batch-failures:${batchId}`;

function isFailure(value: unknown): value is BatchFailure {
  if (!value || typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  const e = v.error as Record<string, unknown> | null | undefined;
  return (
    typeof v.owner === 'string' &&
    typeof v.repo === 'string' &&
    typeof v.base_branch === 'string' &&
    !!e &&
    typeof e === 'object' &&
    typeof e.message === 'string'
  );
}

export function parseFailures(value: unknown): BatchFailure[] {
  return Array.isArray(value) ? value.filter(isFailure) : [];
}

export function storeBatchFailures(batchId: string, failures: BatchFailure[]): void {
  try {
    if (failures.length > 0) sessionStorage.setItem(storageKey(batchId), JSON.stringify(failures));
  } catch {
    // Storage may be unavailable (private mode, quota); the router state still carries them.
  }
}

export function readBatchFailures(batchId: string): BatchFailure[] {
  try {
    const raw = sessionStorage.getItem(storageKey(batchId));
    return raw ? parseFailures(JSON.parse(raw)) : [];
  } catch {
    return [];
  }
}
