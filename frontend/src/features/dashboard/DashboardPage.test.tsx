import { screen, waitFor, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { describe, expect, it } from 'vitest';

import type { JobSummary } from '../../api/types';
import * as f from '../../test/fixtures';
import { renderApp } from '../../test/render';
import { server } from '../../test/server';

const counts = { all: 7, queued: 1, running: 2, succeeded: 3, failed: 1, cancelled: 0 };

function summary(id: string, overrides: Parameters<typeof f.makeJob>[0] = {}): JobSummary {
  return f.toSummary(f.makeJob({ id, short_id: id.slice(-4), ...overrides }));
}

function serveJobs(items: JobSummary[]) {
  const urls: URL[] = [];
  server.use(
    http.get('/api/bounded-context/jobs', ({ request }) => {
      const url = new URL(request.url);
      urls.push(url);
      return HttpResponse.json({ items, page: 1, per_page: 20, has_more: false, total: items.length, counts });
    }),
  );
  return urls;
}

describe('Dashboard', () => {
  it('filters by clicking status tiles that show counts', async () => {
    const urls = serveJobs([summary('aaaaaaaa-0000-4000-8000-000000000001', { status: 'failed', stage: 'running_agent' })]);
    const { user } = renderApp('/');

    const tiles = await screen.findByRole('group', { name: 'Filter by status' });
    const failed = within(tiles).getByRole('button', { name: /Failed/ });
    await waitFor(() => expect(failed).toHaveTextContent('1'));
    expect(within(tiles).getByRole('button', { name: /All/ })).toHaveAttribute('aria-pressed', 'true');
    expect(within(tiles).getByRole('button', { name: /Succeeded/ })).toHaveTextContent('3');

    await user.click(failed);
    expect(failed).toHaveAttribute('aria-pressed', 'true');
    await waitFor(() => expect(urls.at(-1)?.searchParams.get('status')).toBe('failed'));
    expect(screen.getByTestId('location')).toHaveTextContent('/?status=failed');
    // Non-administrators never see the scope switch.
    expect(screen.queryByRole('group', { name: 'Whose jobs' })).not.toBeInTheDocument();
  });

  it('lets administrators list every user\u2019s jobs and filter by requester', async () => {
    server.use(http.get('/api/auth/me', () => HttpResponse.json({ user: { ...f.user, is_admin: true } })));
    const urls = serveJobs([
      summary('aaaaaaaa-0000-4000-8000-000000000002', { requested_by: 'alice_ATT', status: 'succeeded' }),
    ]);
    const { user } = renderApp('/');

    const scope = await screen.findByRole('group', { name: 'Whose jobs' });
    await user.click(within(scope).getByRole('button', { name: 'All users' }));
    expect(await screen.findByRole('heading', { name: 'All generations' })).toBeInTheDocument();
    await waitFor(() => expect(urls.at(-1)?.searchParams.get('scope')).toBe('all'));

    const table = await screen.findByRole('table');
    expect(within(table).getByRole('columnheader', { name: 'Requested by' })).toBeInTheDocument();
    await user.click(within(table).getByRole('button', { name: 'alice_ATT' }));
    await waitFor(() => expect(urls.at(-1)?.searchParams.get('requested_by')).toBe('alice_ATT'));
    expect(screen.getByRole('searchbox', { name: 'Requested by' })).toHaveValue('alice_ATT');
  });

  it('searches jobs and pages through the results', async () => {
    const urls: URL[] = [];
    server.use(
      http.get('/api/bounded-context/jobs', ({ request }) => {
        const url = new URL(request.url);
        urls.push(url);
        const page = Number(url.searchParams.get('page') ?? '1');
        const items = [summary(`aaaaaaaa-0000-4000-8000-00000000000${page}`)];
        return HttpResponse.json({ items, page, per_page: 20, has_more: page < 3, total: 45, counts });
      }),
    );
    const { user } = renderApp('/');

    const pages = await screen.findByRole('navigation', { name: 'Job list pages' });
    expect(within(pages).getByText('Showing 1–20 of 45')).toBeInTheDocument();
    expect(within(pages).getByRole('button', { name: 'Page 1' })).toHaveAttribute('aria-current', 'page');
    await user.click(within(pages).getByRole('button', { name: 'Page 3' }));
    await waitFor(() => expect(urls.at(-1)?.searchParams.get('page')).toBe('3'));
    expect(within(pages).getByText('Showing 41–45 of 45')).toBeInTheDocument();

    await user.type(screen.getByRole('searchbox', { name: 'Search jobs' }), 'shop');
    await waitFor(() => expect(urls.at(-1)?.searchParams.get('q')).toBe('shop'));
    // A new search starts again from the first page.
    expect(urls.at(-1)?.searchParams.get('page')).toBe('1');
    expect(screen.getByTestId('location')).toHaveTextContent('/?q=shop');

    await user.selectOptions(within(pages).getByLabelText('Rows per page'), '50');
    await waitFor(() => expect(urls.at(-1)?.searchParams.get('per_page')).toBe('50'));
  });
});
