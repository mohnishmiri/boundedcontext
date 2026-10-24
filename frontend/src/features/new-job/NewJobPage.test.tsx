import { screen, waitFor, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { describe, expect, it } from 'vitest';

import type { CreateJobRequest } from '../../api/types';
import * as f from '../../test/fixtures';
import { renderApp } from '../../test/render';
import { apiError, server } from '../../test/server';

const NEW_ID = '7f000000-2222-4c4c-9d9d-000000000001';

function captureCreate(response: () => Response = () => HttpResponse.json(f.makeJob({ id: NEW_ID }), { status: 202 })) {
  const bodies: CreateJobRequest[] = [];
  server.use(
    http.post('/api/bounded-context/jobs', async ({ request }) => {
      bodies.push((await request.json()) as CreateJobRequest);
      return response();
    }),
  );
  return bodies;
}

async function chooseRepository(user: ReturnType<typeof renderApp>['user']) {
  await user.click(await screen.findByRole('radio', { name: /acme Organization/ }));
  await user.click(await screen.findByRole('radio', { name: /acme\/shop/ }));
  await screen.findByText(/You can generate documentation and open a pull request/);
}

describe('New generation', () => {
  it('walks through account, repository, branch and model, then starts the job', async () => {
    const bodies = captureCreate();
    const { user } = renderApp('/jobs/new');

    await chooseRepository(user);

    // Default branch is preselected.
    const branchFieldset = screen.getByRole('group', { name: '3. Base branch' });
    expect(within(branchFieldset).getByText(/default branch/)).toBeInTheDocument();
    expect(await within(branchFieldset).findByRole('radio', { name: /main/ })).toBeChecked();

    // Default model is preselected.
    const model = screen.getByLabelText('4. Model') as HTMLSelectElement;
    expect(model.value).toBe('auto');
    expect(model).toBeEnabled();
    expect(screen.getByText(/enforced by GitHub Copilot/)).toBeInTheDocument();

    await user.selectOptions(model, 'claude-haiku-4.5');
    await user.type(screen.getByLabelText('Subdirectory to analyse'), 'services/orders');
    await user.click(screen.getByRole('button', { name: 'Generate documentation' }));

    await waitFor(() => expect(screen.getByTestId('location')).toHaveTextContent(`/jobs/${NEW_ID}`));
    expect(bodies).toEqual([
      {
        owner: 'acme',
        repo: 'shop',
        base_branch: 'main',
        model: 'claude-haiku-4.5',
        inputs: { scope_path: 'services/orders' },
      },
    ]);
  });

  it('offers a link to the existing job on duplicate_job', async () => {
    captureCreate(() =>
      apiError(409, 'duplicate_job', 'A generation for acme/shop@main with this model is already running.', {
        details: { job_id: f.JOB_ID },
      }),
    );
    const { user } = renderApp('/jobs/new');
    await chooseRepository(user);
    await user.click(screen.getByRole('button', { name: 'Generate documentation' }));

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('A matching generation is already running');
    expect(within(alert).getByRole('link', { name: 'Open the existing job' })).toHaveAttribute('href', `/jobs/${f.JOB_ID}`);
  });

  it('shows server validation errors next to the fields', async () => {
    captureCreate(() =>
      apiError(400, 'validation_failed', 'Some fields are invalid', {
        details: { fields: { 'inputs.scope_path': 'Directory does not exist on this branch' } },
      }),
    );
    const { user } = renderApp('/jobs/new');
    await chooseRepository(user);
    await user.type(screen.getByLabelText('Subdirectory to analyse'), 'nope');
    await user.click(screen.getByRole('button', { name: 'Generate documentation' }));

    expect(await screen.findByText('Directory does not exist on this branch')).toBeInTheDocument();
    expect(screen.getByLabelText('Subdirectory to analyse')).toHaveAttribute('aria-invalid', 'true');
  });

  it('validates path inputs before submitting', async () => {
    const bodies = captureCreate();
    const { user } = renderApp('/jobs/new');
    await chooseRepository(user);
    await user.type(screen.getByLabelText('Subdirectory to analyse'), '../etc');
    await user.click(screen.getByRole('button', { name: 'Generate documentation' }));

    expect(await screen.findByText(/Must be a relative path inside the repository/)).toBeInTheDocument();
    expect(bodies).toHaveLength(0);
  });

  it('locks the model selector when the agent pins a model', async () => {
    server.use(
      http.get('/api/models', () =>
        HttpResponse.json({
          ...f.models,
          locked_model: 'claude-haiku-4.5',
          lock_reason: 'The agent file pins model: claude-haiku-4.5.',
        }),
      ),
    );
    const bodies = captureCreate();
    const { user } = renderApp('/jobs/new');
    await chooseRepository(user);

    const model = screen.getByLabelText('4. Model') as HTMLSelectElement;
    expect(model).toBeDisabled();
    expect(model.value).toBe('claude-haiku-4.5');
    expect(screen.getByText('The agent file pins model: claude-haiku-4.5.')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Generate documentation' }));
    await waitFor(() => expect(bodies).toHaveLength(1));
    expect(bodies[0].model).toBe('claude-haiku-4.5');
  });

  it('blocks submission when the repository cannot be used', async () => {
    server.use(
      http.get('/api/github/repositories/:owner/:repo', () =>
        HttpResponse.json({ ...f.repositoryDetail, can_generate: false, can_publish: false, reason: 'Archived repository' }),
      ),
    );
    const { user } = renderApp('/jobs/new');
    await user.click(await screen.findByRole('radio', { name: /acme Organization/ }));
    await user.click(await screen.findByRole('radio', { name: /acme\/shop/ }));

    expect(await screen.findByText('Archived repository')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Generate documentation' })).toBeDisabled();
  });

  it('prefills from query parameters', async () => {
    renderApp('/jobs/new?owner=acme&repo=shop&branch=develop&model=claude-haiku-4.5');
    await screen.findByText(/You can generate documentation and open a pull request/);
    const branchFieldset = screen.getByRole('group', { name: '3. Base branch' });
    expect(await within(branchFieldset).findByRole('radio', { name: /develop/ })).toBeChecked();
    expect((screen.getByLabelText('4. Model') as HTMLSelectElement).value).toBe('claude-haiku-4.5');
  });
});
