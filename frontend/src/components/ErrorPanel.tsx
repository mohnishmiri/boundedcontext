import type { ReactNode } from 'react';

import { ApiError } from '../api/client';
import type { JobError } from '../api/types';
import { ERROR_GUIDANCE } from '../lib/errors';
import { Alert } from './Alert';

type Props = {
  error: JobError | ApiError | Error;
  title?: string;
  children?: ReactNode;
};

/** Shows what failed (server message), what to do (guidance per code) and whether retrying may help. */
export function ErrorPanel({ error, title = 'Something went wrong', children }: Props) {
  const code = 'code' in error ? error.code : undefined;
  const retryable = 'retryable' in error ? error.retryable : false;
  const guidance = code ? ERROR_GUIDANCE[code] : undefined;
  const requestId = error instanceof ApiError ? error.requestId : null;
  return (
    <Alert tone="error" title={title} role="alert">
      <p>{error.message}</p>
      {guidance && guidance !== error.message && <p>{guidance}</p>}
      {retryable && <p className="text-xs opacity-80">This is usually temporary — trying again may succeed.</p>}
      {children}
      {(code || requestId) && (
        <p className="font-mono text-xs opacity-70">
          {code && <span>code: {code}</span>}
          {code && requestId && ' · '}
          {requestId && <span>request id: {requestId}</span>}
        </p>
      )}
    </Alert>
  );
}
