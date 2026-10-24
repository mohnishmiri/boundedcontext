import { screen, waitFor, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, it } from 'vitest';

import type { JobSummary } from '../../api/types';
import * as f from '../../test/fixtures';
import { renderApp } from '../../test/render';
import { apiError, server } from '../../test/server';
import { storeBatchFailures } from './batchFailures';

const ID_A = '7f000000-2222-4c4c-9d9d-00000000000a';
const ID_B = '7f000000-2222-4c4c-9d9d-00000000000b';

function summary(id: string, overrides: Parameters<typeof f.makeJob>[0] = {}): JobSummary {
  return f.toSummary(f.makeJob({ id, short_id: id.slice(-4), batch_id: f.BATCH_ID, ...overrides }));
}

function serveBatch(respond: (url: URL) => JobSummary[]) {
  const urls: URL[] = [];
  server.use(
    http.get('/api/bounded-context/jobs', ({ request }) => {
      const url = new URL(request.url);
      urls.push(url);
      return HttpResponse.json({ items: respond(url), page: 1, per_page: 100, has_more: false });
    }),
  );
  return urls;
}

beforeEach(() => sessionStorage.clear());

describe('Batch page', () => {
  it('lists the jobs of the batch with status badges and links', async () => {
    const urls = serveBatch(() => [
      summary(ID_A, { status: 'succeeded', stage: 'completed', publish_status: 'not_started' }),
      summary(ID_B, { repo: 'billing', status: 'failed', stage: 'running_agent' }),
    ]);
    renderApp(`/batches/${f.BATCH_ID}`);

    const table = await screen.findByRole('table');
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows).toHaveLength(2);
    expect(within(rows[0]).getByRole('link', { name: /acme\/shop/ })).toHaveAttribute('href', `/jobs/${ID_A}`);
    expect(within(rows[0]).getByText('Succeeded')).toBeInTheDocument();
    expect(within(rows[1]).getByRole('link', { name: /acme\/billing/ })).toHaveAttribute('href', `/jobs/${ID_B}`);
    expect(within(rows[1]).getByText('Failed')).toBeInTheDocument();
    expect(screen.getByText('2 of 2 jobs finished.')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: /Not started/ })).not.toBeInTheDocument();

    expect(urls[0].searchParams.get('batch_id')).toBe(f.BATCH_ID);
    expect(urls[0].searchParams.get('per_page')).toBe('100');
  });

  it('polls until every job is terminal', async () => {
    let calls = 0;
    const urls = serveBatch(() => {
      calls += 1;
      return [
        summary(ID_A, calls === 1 ? { status: 'running', stage: 'running_agent' } : { status: 'succeeded', stage: 'completed' }),
      ];
    });
    renderApp(`/batches/${f.BATCH_ID}`);

    expect(await screen.findByText('Running')).toBeInTheDocument();
    expect(screen.getByText('0 of 1 job finished.')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText('Succeeded')).toBeInTheDocument(), { timeout: 6000 });
    expect(screen.getByText('1 of 1 job finished.')).toBeInTheDocument();

    // Polling stops once everything is done.
    const settled = urls.length;
    await new Promise((r) => setTimeout(r, 3500));
    expect(urls).toHaveLength(settled);
  }, 15_000);

  it('lists the failures remembered for the batch', async () => {
    serveBatch(() => [summary(ID_A, { status: 'succeeded', stage: 'completed' })]);
    storeBatchFailures(f.BATCH_ID, [
      {
        owner: 'acme',
        repo: 'billing',
        base_branch: 'main',
        error: { code: 'repository_access_denied', message: 'You cannot read acme/billing.' },
      },
      {
        owner: 'acme',
        repo: 'orders',
        base_branch: 'develop',
        error: { code: 'duplicate_job', message: 'Already running.', details: { job_id: f.JOB_ID } },
      },
    ]);
    renderApp(`/batches/${f.BATCH_ID}`);

    expect(await screen.findByRole('heading', { name: 'Not started (2)' })).toBeInTheDocument();
    expect(screen.getByText('You cannot read acme/billing.')).toBeInTheDocument();
    expect(screen.getByText('Already running.')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Open the existing job' })).toHaveAttribute('href', `/jobs/${f.JOB_ID}`);
  });

  it('shows an error when the batch cannot be loaded', async () => {
    server.use(http.get('/api/bounded-context/jobs', () => apiError(404, 'not_found', 'Batch not found.')));
    renderApp(`/batches/${f.BATCH_ID}`);
    expect(await screen.findByText('Could not load the batch')).toBeInTheDocument();
  });
});
