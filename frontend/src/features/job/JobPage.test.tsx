import { act, screen, waitFor, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Job } from '../../api/types';
import * as f from '../../test/fixtures';
import { MockEventSource } from '../../test/MockEventSource';
import { renderApp } from '../../test/render';
import { server } from '../../test/server';

function serveJob(job: Job) {
  server.use(http.get('/api/bounded-context/jobs/:id', () => HttpResponse.json(job)));
}

beforeEach(() => {
  MockEventSource.reset();
  vi.stubGlobal('EventSource', MockEventSource);
});

describe('Job page', () => {
  it('shows generation steps and an actionable error for a failed job, and retries', async () => {
    serveJob(f.failedJob());
    const retried = f.makeJob({ id: '99999999-0000-4000-8000-000000000000' });
    server.use(http.post('/api/bounded-context/jobs/:id/retry', () => HttpResponse.json(retried, { status: 202 })));
    const { user } = renderApp(`/jobs/${f.JOB_ID}`);

    const steps = await screen.findByRole('list', { name: 'Generation steps' });
    const items = within(steps).getAllByRole('listitem');
    expect(items.map((li) => li.getAttribute('data-state'))).toEqual(['done', 'done', 'done', 'failed', 'skipped']);
    expect(within(items[3]).getByText('Running bounded-context agent')).toBeInTheDocument();

    const panel = screen.getByRole('alert');
    expect(panel).toHaveTextContent('Generation failed');
    expect(panel).toHaveTextContent('The agent did not finish within 20 minutes.');
    expect(panel).toHaveTextContent(/did not finish within the time limit/);
    expect(panel).toHaveTextContent('code: agent_timeout');
    // Terminal job: no live stream.
    expect(MockEventSource.instances).toHaveLength(0);

    await user.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(screen.getByTestId('location')).toHaveTextContent(`/jobs/${retried.id}`));
  });

  it('follows a running job over SSE and closes the stream when it finishes', async () => {
    serveJob(f.runningJob());
    const { user } = renderApp(`/jobs/${f.JOB_ID}`);
    await screen.findByRole('list', { name: 'Generation steps' });

    const es = MockEventSource.latest();
    expect(es.url).toBe(`/api/bounded-context/jobs/${f.JOB_ID}/events`);
    expect(screen.getByText('Live')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: /Logs/ }));
    act(() => {
      es.emit('open');
      es.emit('log', { seq: 41, ts: '2026-10-24T10:02:00Z', level: 'info', stage: 'running_agent', message: 'Reading src/orders' });
      es.emit('ping');
    });
    expect(await screen.findByText('Reading src/orders')).toBeInTheDocument();

    act(() => es.emit('job', f.succeededJob()));
    expect(es.closed).toBe(true);
    expect(await screen.findByRole('heading', { name: 'Review generated documentation' })).toBeInTheDocument();
    expect(await screen.findByRole('heading', { name: 'Bounded Contexts — shop' })).toBeInTheDocument();
    expect(MockEventSource.instances).toHaveLength(1);
  });

  it('shows preview, diff and never-committed changes, then publishes a pull request', async () => {
    serveJob(f.succeededJob());
    const queued = f.succeededJob({
      can_publish: false,
      publish: {
        ...f.succeededJob().publish,
        status: 'queued',
        stage: 'queued',
        steps: [
          { key: 'checking_base', label: 'Checking base branch', state: 'pending', started_at: null, finished_at: null },
          { key: 'creating_commit', label: 'Creating commit', state: 'pending', started_at: null, finished_at: null },
        ],
      },
    });
    let publishCalls = 0;
    server.use(
      http.post('/api/bounded-context/jobs/:id/pull-request', ({ request }) => {
        publishCalls += 1;
        expect(request.headers.get('X-Requested-With')).toBe('bcp');
        return HttpResponse.json(queued, { status: 202 });
      }),
    );
    const { user } = renderApp(`/jobs/${f.JOB_ID}`);

    expect(await screen.findByRole('heading', { name: 'Bounded Contexts — shop' })).toBeInTheDocument();
    const warning = screen.getByText(/never committed/).closest('div') as HTMLElement;
    expect(warning.parentElement).toHaveTextContent('README.md');

    await user.click(screen.getByRole('tab', { name: 'Diff' }));
    const table = screen.getByRole('table');
    expect(table.querySelectorAll('tr[data-kind="add"]')).toHaveLength(5);
    expect(screen.getAllByRole('link', { name: /Download/ })[0]).toHaveAttribute(
      'href',
      `/api/bounded-context/jobs/${f.JOB_ID}/output`,
    );

    // Terminal + publish not started: no stream until publishing starts.
    expect(MockEventSource.instances).toHaveLength(0);
    await user.click(screen.getByRole('button', { name: 'Create pull request' }));
    await waitFor(() => expect(publishCalls).toBe(1));
    expect(await screen.findByRole('list', { name: 'Publishing steps' })).toBeInTheDocument();

    await waitFor(() => expect(MockEventSource.instances).toHaveLength(1));
    const es = MockEventSource.latest();
    act(() =>
      es.emit(
        'job',
        f.succeededJob({
          can_publish: false,
          publish_status: 'succeeded',
          pr_url: 'https://github.com/acme/shop/pull/7',
          publish: {
            ...queued.publish,
            status: 'succeeded',
            stage: 'completed',
            branch: 'bounded-context/shop-0b9c2f8e',
            commit_sha: 'c0ffee0000000000000000000000000000000000',
            commit_url: 'https://github.com/acme/shop/commit/c0ffee0000000000000000000000000000000000',
            pr_number: 7,
            pr_url: 'https://github.com/acme/shop/pull/7',
          },
        }),
      ),
    );
    expect(es.closed).toBe(true);
    expect(await screen.findByRole('link', { name: 'Open pull request #7' })).toHaveAttribute(
      'href',
      'https://github.com/acme/shop/pull/7',
    );
    expect(screen.getByRole('link', { name: 'bounded-context/shop-0b9c2f8e' })).toHaveAttribute(
      'href',
      'https://github.com/acme/shop/tree/bounded-context/shop-0b9c2f8e',
    );
  });

  it('explains base_branch_changed and links to a prefilled new generation', async () => {
    serveJob(
      f.succeededJob({
        publish: {
          ...f.succeededJob().publish,
          status: 'failed',
          stage: 'checking_base',
          error: { code: 'base_branch_changed', message: 'main moved and docs/bounded-context.md changed.', retryable: false },
        },
      }),
    );
    renderApp(`/jobs/${f.JOB_ID}`);
    expect(await screen.findByText('main moved and docs/bounded-context.md changed.')).toBeInTheDocument();
    expect(screen.getByText(/Start a new generation on the latest commit/)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Generate again on the latest commit' })).toHaveAttribute(
      'href',
      '/jobs/new?owner=acme&repo=shop&branch=main&model=auto',
    );
  });

  it('shows a not-found page for unknown jobs', async () => {
    server.use(
      http.get('/api/bounded-context/jobs/:id', () =>
        HttpResponse.json(
          { error: { code: 'not_found', message: 'job not found', retryable: false, request_id: 'r' } },
          { status: 404 },
        ),
      ),
    );
    renderApp('/jobs/does-not-exist');
    expect(await screen.findByRole('heading', { name: 'Job not found' })).toBeInTheDocument();
  });
});
