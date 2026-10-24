import { QueryClient } from '@tanstack/react-query';

import { isApiError } from '../api/client';

export function createQueryClient(opts: { retry?: boolean } = {}): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: (failureCount, error) => {
          if (opts.retry === false) return false;
          if (isApiError(error)) return error.retryable && failureCount < 2;
          return failureCount < 1;
        },
        refetchOnWindowFocus: false,
      },
      mutations: { retry: false },
    },
  });
}
