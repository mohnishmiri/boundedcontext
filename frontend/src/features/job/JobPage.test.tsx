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
  it('publishes to a custom central repository instead of the generator default', async () => {
    serveJob(
      f.succeededJob({
        publish: { ...f.succeededJob().publish, central_available: true, central_repo: 'ATT-DP1/default-docs' },
      }),
    );
    let body: unknown;
    server.use(
      http.post('/api/bounded-context/jobs/:id/pull-request', async ({ request }) => {
        body = await request.json();
        return HttpResponse.json(f.succeededJob(), { status: 202 });
      }),
    );
    const { user } = renderApp(`/jobs/${f.JOB_ID}`);

    await user.type(await screen.findByRole('textbox', { name: 'Pull request title' }), 'Docs');
    await user.click(screen.getByRole('checkbox', { name: /Also create a pull request/ }));
    const repo = screen.getByRole('textbox', { name: 'Central repository' });
    expect(repo).toHaveValue('ATT-DP1/default-docs');

    await user.clear(repo);
    await user.type(repo, 'not a repo');
    expect(screen.getByRole('button', { name: 'Create pull request' })).toBeDisabled();
    expect(screen.getByText('Enter the repository as owner/repository.')).toBeInTheDocument();

    await user.clear(repo);
    await user.type(repo, 'ATT-DP5/team-docs');
    await user.click(screen.getByRole('button', { name: 'Create pull request' }));
    await waitFor(() =>
      expect(body).toEqual({ title: 'Docs', publish_central: true, reviewers: [], central_repo: 'ATT-DP5/team-docs' }),
    );
  });

  it('shows another user\u2019s job read-only to an administrator', async () => {
    serveJob(
      f.succeededJob({ viewer_is_owner: false, requested_by: 'someone_ATT', can_publish: false, can_retry: false }),
    );
    renderApp(`/jobs/${f.JOB_ID}`);

    expect(await screen.findByText('Administrator view')).toBeInTheDocument();
    expect(screen.getByText('someone_ATT', { selector: 'strong' })).toBeInTheDocument();
    expect(screen.getByText('No pull request has been opened for this job yet.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Create pull request' })).not.toBeInTheDocument();
    expect(screen.getAllByRole('link', { name: 'Download all (.zip)' }).length).toBeGreaterThan(0);
  });

  it('lets an administrator open the pull request on behalf of the requester', async () => {
    serveJob(f.succeededJob({ viewer_is_owner: false, requested_by: 'someone_ATT', can_publish: true }));
    let body: unknown;
    server.use(
      http.post('/api/bounded-context/jobs/:id/pull-request', async ({ request }) => {
        body = await request.json();
        return HttpResponse.json(f.succeededJob({ viewer_is_owner: false }), { status: 202 });
      }),
    );
    const { user } = renderApp(`/jobs/${f.JOB_ID}`);

    expect(await screen.findByText('Publishing on behalf of the requester')).toBeInTheDocument();
    await user.type(screen.getByRole('textbox', { name: 'Pull request title' }), 'Docs');
    await user.click(screen.getByRole('button', { name: 'Create pull request for someone_ATT' }));
    await waitFor(() => expect(body).toEqual({ title: 'Docs', publish_central: false, reviewers: [] }));
  });
  it('shows the generator and a link to the batch', async () => {
    serveJob(
      f.failedJob({
        batch_id: f.BATCH_ID,
        generator: {
          id: 'g1',
          name: 'Team generator',
          source: 'user',
          owner: 'acme',
          repo: 'bc-gen',
          branch: 'main',
          path: null,
          sha: 'abcdef1234567890',
        },
      }),
    );
    renderApp(`/jobs/${f.JOB_ID}`);

    expect(await screen.findByText('Team generator')).toHaveAttribute('title', 'acme/bc-gen');
    expect(screen.getByText('(user)')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Batch' })).toHaveAttribute('href', `/batches/${f.BATCH_ID}`);
  });

  it('shows generation steps and an actionable error for a failed job, and retries', async () => {
    serveJob(f.failedJob());
    const retried = f.makeJob({ id: '99999999-0000-4000-8000-000000000000' });
    server.use(http.post('/api/bounded-context/jobs/:id/retry', () => HttpResponse.json(retried, { status: 202 })));
    const { user } = renderApp(`/jobs/${f.JOB_ID}`);

    const steps = await screen.findByRole('list', { name: 'Generation steps' });
    // Logs are open by default, also for finished jobs.
    expect(screen.getByRole('button', { name: /Logs/ })).toHaveAttribute('aria-expanded', 'true');
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
    renderApp(`/jobs/${f.JOB_ID}`);
    await screen.findByRole('list', { name: 'Generation steps' });

    const es = MockEventSource.latest();
    expect(es.url).toBe(`/api/bounded-context/jobs/${f.JOB_ID}/events`);
    expect(screen.getByText('Live')).toBeInTheDocument();

    // Logs are open and streaming while the job runs.
    expect(screen.getByRole('button', { name: /Logs/ })).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('Streaming')).toBeInTheDocument();
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
    serveJob(
      f.succeededJob({
        publish: {
          ...f.succeededJob().publish,
          central_available: true,
          central_repo: 'ATT-DP1/apm0014313-ragaflow-ai-context-docs',
        },
      }),
    );
    const queued = f.succeededJob({
      can_publish: false,
      publish: {
        ...f.succeededJob().publish,
        central_available: true,
        central_repo: 'ATT-DP1/apm0014313-ragaflow-ai-context-docs',
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
      http.post('/api/bounded-context/jobs/:id/pull-request', async ({ request }) => {
        publishCalls += 1;
        expect(request.headers.get('X-Requested-With')).toBe('bcp');
        expect(await request.json()).toEqual({
          title: 'APM-123: Update shop context',
          publish_central: true,
          reviewers: ['alice', 'bob'],
        });
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
    for (const zip of screen.getAllByRole('link', { name: 'Download all (.zip)' })) {
      expect(zip).toHaveAttribute('href', `/api/bounded-context/jobs/${f.JOB_ID}/archive`);
    }
    expect(screen.getByRole('link', { name: /^Download .*\.md$/ })).toHaveAttribute(
      'href',
      `/api/bounded-context/jobs/${f.JOB_ID}/output`,
    );

    // Terminal + publish not started: no stream until publishing starts.
    expect(MockEventSource.instances).toHaveLength(0);
    await user.type(screen.getByRole('textbox', { name: 'Pull request title' }), 'APM-123: Update shop context');
    await user.type(screen.getByRole('textbox', { name: /Reviewers/ }), 'alice, @bob');
    await user.click(screen.getByRole('checkbox', { name: /Also create a pull request/ }));
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
            central_pr_number: 12,
            central_pr_url: 'https://github.com/ATT-DP1/apm0014313-ragaflow-ai-context-docs/pull/12',
          },
        }),
      ),
    );
    expect(es.closed).toBe(true);
    expect(await screen.findByRole('link', { name: 'Open pull request #7' })).toHaveAttribute(
      'href',
      'https://github.com/acme/shop/pull/7',
    );
    expect(screen.getByRole('link', { name: 'Open central repository pull request #12' })).toHaveAttribute(
      'href',
      'https://github.com/ATT-DP1/apm0014313-ragaflow-ai-context-docs/pull/12',
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
