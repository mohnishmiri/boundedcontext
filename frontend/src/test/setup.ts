import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterAll, afterEach, beforeAll, vi } from 'vitest';

import { server } from './server';

/**
 * Node's fetch (undici) rejects jsdom's AbortSignal ("Expected signal to be an instance of AbortSignal"), which
 * react-query passes to every query. Strip the signal before it reaches fetch and honour aborts by rejecting.
 * Installed after msw patches fetch so msw sees the signal-free request. Test-environment only.
 */
let fetchBeforeShim: typeof fetch | undefined;

function installAbortSignalShim() {
  const inner = globalThis.fetch;
  fetchBeforeShim = inner;
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    if (!init?.signal) return inner(input, init);
    const { signal, ...rest } = init;
    if (signal.aborted) return Promise.reject(new DOMException('Aborted', 'AbortError'));
    return new Promise<Response>((resolve, reject) => {
      signal.addEventListener('abort', () => reject(new DOMException('Aborted', 'AbortError')), { once: true });
      inner(input, rest).then(resolve, reject);
    });
  }) as typeof fetch;
}

beforeAll(() => {
  server.listen({ onUnhandledRequest: 'error' });
  installAbortSignalShim();
});
afterEach(() => {
  cleanup();
  server.resetHandlers();
  vi.unstubAllGlobals();
});
afterAll(() => {
  if (fetchBeforeShim) globalThis.fetch = fetchBeforeShim;
  server.close();
});

if (typeof window.matchMedia !== 'function') {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => undefined,
      removeListener: () => undefined,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      dispatchEvent: () => false,
    }),
  });
}
