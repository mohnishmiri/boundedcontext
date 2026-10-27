import { screen, waitFor, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { describe, expect, it } from 'vitest';

import type {
  CreateBatchRequest,
  CreateGeneratorRequest,
  Generator,
  GeneratorAgentInfo,
  Repository,
} from '../../api/types';
import * as f from '../../test/fixtures';
import { renderApp } from '../../test/render';
import { apiError, server } from '../../test/server';

const NEW_ID = '7f000000-2222-4c4c-9d9d-000000000001';
const NEW_ID_2 = '7f000000-2222-4c4c-9d9d-000000000002';

type User = ReturnType<typeof renderApp>['user'];

function summary(id: string, overrides: Parameters<typeof f.makeJob>[0] = {}) {
  return f.toSummary(f.makeJob({ id, short_id: id.slice(-4), batch_id: f.BATCH_ID, ...overrides }));
}

function captureBatch(
  response: () => Response = () =>
    HttpResponse.json(f.makeBatchResponse({ jobs: [summary(NEW_ID)] }), { status: 202 }),
) {
  const bodies: CreateBatchRequest[] = [];
  server.use(
    http.post('/api/bounded-context/batches', async ({ request }) => {
      bodies.push((await request.json()) as CreateBatchRequest);
      return response();
    }),
  );
  return bodies;
}

async function chooseAccount(user: User, name: RegExp = /acme Organization/) {
  await user.click(await screen.findByRole('radio', { name }));
}

async function tick(user: User, repoName: RegExp) {
  await user.click(await screen.findByRole('checkbox', { name: repoName }));
}

async function chooseRepository(user: User) {
  await chooseAccount(user);
  await tick(user, /acme\/shop/);
  const branch = await screen.findByLabelText('Base branch for acme/shop');
  // Wait until the repository access check finished, so the submit button can become enabled.
  await waitFor(() => expect(screen.queryByText(/Checking access to acme\/shop/)).not.toBeInTheDocument());
  return branch as HTMLSelectElement;
}

const submitButton = () => screen.getByRole('button', { name: 'Generate documentation' });

describe('New generation', () => {
  it('walks through generator, account, repository and model, then starts a single job', async () => {
    const bodies = captureBatch();
    const { user } = renderApp('/jobs/new');

    // The built-in generator is preselected and described.
    const generator = await screen.findByRole('radio', { name: /Bounded Context Generator/ });
    expect(generator).toBeChecked();
    expect(screen.getByText('Writes a bounded-context document.')).toBeInTheDocument();
    expect(await screen.findByText(/Placeholder generator/)).toBeInTheDocument();

    const branch = await chooseRepository(user);
    // Default branch is preselected.
    await waitFor(() => expect(branch.value).toBe('main'));
    expect(within(branch).getByRole('option', { name: 'main (default)' })).toBeInTheDocument();

    // Default model is preselected.
    const model = screen.getByLabelText('4. Model') as HTMLSelectElement;
    expect(model.value).toBe('auto');
    expect(model).toBeEnabled();
    expect(screen.getByText(/enforced by GitHub Copilot/)).toBeInTheDocument();

    await user.selectOptions(model, 'claude-haiku-4.5');
    await user.type(screen.getByLabelText('Subdirectory to analyse'), 'services/orders');
    await user.click(submitButton());

    await waitFor(() => expect(screen.getByTestId('location')).toHaveTextContent(`/jobs/${NEW_ID}`));
    expect(bodies).toEqual([
      {
        generator_id: f.GENERATOR_ID,
        model: 'claude-haiku-4.5',
        inputs: { scope_path: 'services/orders' },
        repositories: [{ owner: 'acme', repo: 'shop', base_branch: 'main' }],
      },
    ]);
  });

  it('keeps the selection across organisation switches and searches, with a branch per repository', async () => {
    const bodies = captureBatch(() =>
      HttpResponse.json(
        f.makeBatchResponse({
          jobs: [summary(NEW_ID), summary(NEW_ID_2, { owner: 'octocat', repo: 'notes', base_branch: 'trunk' })],
          failures: [
            {
              owner: 'acme',
              repo: 'billing',
              base_branch: 'main',
              error: { code: 'app_not_installed', message: 'The GitHub App is not installed on acme/billing.' },
            },
          ],
        }),
        { status: 202 },
      ),
    );
    const { user } = renderApp('/jobs/new');

    await chooseAccount(user);
    await tick(user, /acme\/shop/);
    await tick(user, /acme\/billing/);
    expect(await screen.findByText('Selected (2)')).toBeInTheDocument();

    // Search narrows the list but not the selection.
    await user.type(screen.getByLabelText('Search repositories in acme'), 'bill');
    await waitFor(() => expect(screen.queryByRole('checkbox', { name: /acme\/shop/ })).not.toBeInTheDocument());
    expect(screen.getByRole('checkbox', { name: /acme\/billing/ })).toBeChecked();
    expect(screen.getByText('Selected (2)')).toBeInTheDocument();

    // Switch organisation and add one there.
    await chooseAccount(user, /octocat Personal account/);
    await tick(user, /octocat\/notes/);
    expect(await screen.findByText('Selected (3)')).toBeInTheDocument();
    const selected = screen.getByRole('list', { name: 'Selected repositories' });
    expect(within(selected).getAllByRole('listitem')).toHaveLength(3);

    // Back to acme: ticks are still there.
    await chooseAccount(user);
    expect(await screen.findByRole('checkbox', { name: /acme\/shop/ })).toBeChecked();

    // Per-repository branch: defaults come from the repository, and can be changed.
    expect(within(selected).getByLabelText('Base branch for octocat/notes')).toHaveValue('trunk');
    const shopBranch = within(selected).getByLabelText('Base branch for acme/shop') as HTMLSelectElement;
    await waitFor(() => expect(within(shopBranch).getByRole('option', { name: 'develop' })).toBeInTheDocument());
    await user.selectOptions(shopBranch, 'develop');

    // Remove one.
    await user.click(within(selected).getByRole('button', { name: 'Remove acme/billing' }));
    expect(screen.getByText('Selected (2)')).toBeInTheDocument();
    await waitFor(() => expect(submitButton()).toBeEnabled());

    await user.click(submitButton());
    await waitFor(() => expect(screen.getByTestId('location')).toHaveTextContent(`/batches/${f.BATCH_ID}`));
    expect(bodies).toEqual([
      {
        generator_id: f.GENERATOR_ID,
        model: 'auto',
        inputs: {},
        repositories: [
          { owner: 'acme', repo: 'shop', base_branch: 'develop' },
          { owner: 'octocat', repo: 'notes', base_branch: 'trunk' },
        ],
      },
    ]);
  });

  it('opens the batch page with the failures when some repositories were rejected', async () => {
    captureBatch(() =>
      HttpResponse.json(
        f.makeBatchResponse({
          jobs: [summary(NEW_ID)],
          failures: [
            {
              owner: 'acme',
              repo: 'billing',
              base_branch: 'main',
              error: { code: 'app_not_installed', message: 'The GitHub App is not installed on acme/billing.' },
            },
          ],
        }),
        { status: 202 },
      ),
    );
    server.use(
      http.get('/api/bounded-context/jobs', () =>
        HttpResponse.json({ items: [summary(NEW_ID)], page: 1, per_page: 100, has_more: false }),
      ),
    );
    const { user } = renderApp('/jobs/new');
    await chooseRepository(user);
    await user.click(submitButton());

    await waitFor(() => expect(screen.getByTestId('location')).toHaveTextContent(`/batches/${f.BATCH_ID}`));
    expect(await screen.findByRole('heading', { name: 'Not started (1)' })).toBeInTheDocument();
    expect(screen.getByText('The GitHub App is not installed on acme/billing.')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /acme\/shop/ })).toHaveAttribute('href', `/jobs/${NEW_ID}`);
  });

  it('limits a batch to 25 repositories', async () => {
    const many: Repository[] = Array.from({ length: 26 }, (_, i) => ({
      ...f.repository,
      id: 100 + i,
      name: `svc-${String(i).padStart(2, '0')}`,
      full_name: `acme/svc-${String(i).padStart(2, '0')}`,
    }));
    server.use(
      http.get('/api/github/repositories', () =>
        HttpResponse.json({ items: many, page: 1, per_page: 30, has_more: false }),
      ),
    );
    const { user } = renderApp('/jobs/new');
    await chooseAccount(user);
    for (const r of many.slice(0, 25)) {
      await user.click(await screen.findByRole('checkbox', { name: new RegExp(r.full_name) }));
    }
    expect(await screen.findByText('Selected (25)')).toBeInTheDocument();
    expect(screen.getByText(/up to 25 repositories per batch/)).toBeInTheDocument();
    expect(screen.getByRole('checkbox', { name: /acme\/svc-25/ })).toBeDisabled();
    expect(screen.getByRole('checkbox', { name: /acme\/svc-00/ })).toBeEnabled();
  }, 30_000);

  it('adds a generator repository and selects it', async () => {
    const added: Generator = {
      id: 'user-gen-1',
      name: 'My generator',
      description: 'Team flavoured docs.',
      source: 'user',
      repository: { owner: 'acme', repo: 'bc-gen', branch: 'main', path: 'agents' },
      removable: true,
    };
    let created = false;
    const bodies: CreateGeneratorRequest[] = [];
    server.use(
      http.get('/api/generators', () =>
        HttpResponse.json({ items: created ? [f.builtinGenerator, added] : [f.builtinGenerator], default_id: f.GENERATOR_ID }),
      ),
      http.post('/api/generators', async ({ request }) => {
        bodies.push((await request.json()) as CreateGeneratorRequest);
        created = true;
        return HttpResponse.json(added, { status: 201 });
      }),
      http.get('/api/generators/:id/agent', ({ params }) =>
        HttpResponse.json(
          params.id === added.id
            ? ({ ...f.agent, id: added.id, name: added.name, placeholder: false, inputs: [] } satisfies GeneratorAgentInfo)
            : f.agent,
        ),
      ),
    );
    const { user } = renderApp('/jobs/new');
    await screen.findByRole('radio', { name: /Bounded Context Generator/ });
    expect(await screen.findByText(/Placeholder generator/)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Add a generator repository' }));
    await user.click(screen.getByRole('button', { name: 'Add generator' }));
    expect(screen.getAllByText('This field is required')).toHaveLength(2);

    await user.type(screen.getByLabelText(/^Owner/), 'acme');
    await user.type(screen.getByLabelText(/^Repository/), 'bc-gen');
    await user.type(screen.getByLabelText('Branch (optional)'), 'main');
    await user.type(screen.getByLabelText('Path in repository (optional)'), 'agents');
    await user.type(screen.getByLabelText('Display name (optional)'), 'My generator');
    await user.click(screen.getByRole('button', { name: 'Add generator' }));

    const radio = await screen.findByRole('radio', { name: /My generator/ });
    await waitFor(() => expect(radio).toBeChecked());
    expect(bodies).toEqual([{ owner: 'acme', repo: 'bc-gen', branch: 'main', path: 'agents', name: 'My generator' }]);
    expect(screen.getByText('Team flavoured docs.')).toBeInTheDocument();
    // Its agent info replaced the built-in one: no placeholder warning, no options.
    await waitFor(() => expect(screen.queryByText(/Placeholder generator/)).not.toBeInTheDocument());
    expect(screen.queryByLabelText('Subdirectory to analyse')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Add generator' })).not.toBeInTheDocument();
  });

  it('explains why a generator repository was refused', async () => {
    server.use(
      http.post('/api/generators', () =>
        apiError(403, 'generator_not_allowed', 'acme/other is not on the generator allowlist.'),
      ),
    );
    const { user } = renderApp('/jobs/new');
    await screen.findByRole('radio', { name: /Bounded Context Generator/ });
    await user.click(screen.getByRole('button', { name: 'Add a generator repository' }));
    await user.type(screen.getByLabelText(/^Owner/), 'acme');
    await user.type(screen.getByLabelText(/^Repository/), 'other');
    await user.click(screen.getByRole('button', { name: 'Add generator' }));

    expect(await screen.findByText('acme/other is not on the generator allowlist.')).toBeInTheDocument();
    // The form stays open so the user can correct it.
    expect(screen.getByRole('button', { name: 'Add generator' })).toBeInTheDocument();
  });

  it('removes a user-added generator and falls back to the default', async () => {
    const mine: Generator = {
      id: 'user-gen-1',
      name: 'My generator',
      description: null,
      source: 'user',
      repository: { owner: 'acme', repo: 'bc-gen', branch: null, path: '' },
      removable: true,
    };
    let removed = false;
    const deleted: string[] = [];
    server.use(
      http.get('/api/generators', () =>
        HttpResponse.json({ items: removed ? [f.builtinGenerator] : [f.builtinGenerator, mine], default_id: f.GENERATOR_ID }),
      ),
      http.delete('/api/generators/:id', ({ params }) => {
        deleted.push(String(params.id));
        removed = true;
        return new HttpResponse(null, { status: 204 });
      }),
    );
    const { user } = renderApp('/jobs/new?generator=user-gen-1');
    const radio = await screen.findByRole('radio', { name: /My generator/ });
    expect(radio).toBeChecked();
    // Built-in generators cannot be removed.
    expect(screen.queryByRole('button', { name: 'Remove generator Bounded Context Generator' })).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Remove generator My generator' }));
    await waitFor(() => expect(screen.queryByRole('radio', { name: /My generator/ })).not.toBeInTheDocument());
    expect(deleted).toEqual(['user-gen-1']);
    expect(screen.getByRole('radio', { name: /Bounded Context Generator/ })).toBeChecked();
  });

  it('shows the field errors of a rejected submission', async () => {
    captureBatch(() =>
      apiError(400, 'validation_failed', 'Some fields are invalid', {
        details: { fields: { 'inputs.scope_path': 'Directory does not exist on this branch' } },
      }),
    );
    const { user } = renderApp('/jobs/new');
    await chooseRepository(user);
    await user.type(screen.getByLabelText('Subdirectory to analyse'), 'nope');
    await user.click(submitButton());

    expect(await screen.findByText('Directory does not exist on this branch')).toBeInTheDocument();
    expect(screen.getByLabelText('Subdirectory to analyse')).toHaveAttribute('aria-invalid', 'true');
  });

  it('validates path inputs before submitting', async () => {
    const bodies = captureBatch();
    const { user } = renderApp('/jobs/new');
    await chooseRepository(user);
    await user.type(screen.getByLabelText('Subdirectory to analyse'), '../etc');
    await user.click(submitButton());

    expect(await screen.findByText(/Must be a relative path inside the repository/)).toBeInTheDocument();
    expect(bodies).toHaveLength(0);
  });

  it('locks the model selector when the generator pins a model', async () => {
    server.use(
      http.get('/api/generators/:id/agent', () =>
        HttpResponse.json({
          ...f.agent,
          locked_model: 'claude-haiku-4.5',
          lock_reason: 'The agent file pins model: claude-haiku-4.5.',
        }),
      ),
    );
    const bodies = captureBatch();
    const { user } = renderApp('/jobs/new');
    await chooseRepository(user);

    const model = (await screen.findByLabelText('4. Model')) as HTMLSelectElement;
    await waitFor(() => expect(model).toBeDisabled());
    expect(model.value).toBe('claude-haiku-4.5');
    expect(screen.getByText('The agent file pins model: claude-haiku-4.5.')).toBeInTheDocument();

    await user.click(submitButton());
    await waitFor(() => expect(bodies).toHaveLength(1));
    expect(bodies[0].model).toBe('claude-haiku-4.5');
  });

  it('does not show a warning when the model list is a static catalog', async () => {
    server.use(
      http.get('/api/models', () =>
        HttpResponse.json({
          ...f.models,
          source: 'catalog',
          warning: 'Could not read your Copilot models; showing a default list.',
        }),
      ),
    );
    renderApp('/jobs/new');
    expect(await screen.findByLabelText('4. Model')).toBeEnabled();
    expect(screen.queryByText(/showing a default list/)).not.toBeInTheDocument();
  });

  it('does not show a model warning for the user’s own Copilot models', async () => {
    renderApp('/jobs/new');
    await screen.findByLabelText('4. Model');
    expect(screen.queryByText(/showing a default list/)).not.toBeInTheDocument();
    expect(screen.getByRole('option', { name: /Claude Haiku 4.5 — Anthropic \(×0.33\)/ })).toBeInTheDocument();
  });

  it('blocks submission when a selected repository cannot be used', async () => {
    server.use(
      http.get('/api/github/repositories/:owner/:repo', () =>
        HttpResponse.json({ ...f.repositoryDetail, can_generate: false, can_publish: false, reason: 'Archived repository' }),
      ),
    );
    const { user } = renderApp('/jobs/new');
    await chooseAccount(user);
    await tick(user, /acme\/shop/);

    expect(await screen.findByText('Archived repository')).toBeInTheDocument();
    expect(submitButton()).toBeDisabled();
  });

  it('shows access errors such as app-not-installed for a selected repository', async () => {
    server.use(
      http.get('/api/github/repositories/:owner/:repo', () =>
        apiError(403, 'app_not_installed', 'The GitHub App is not installed on this repository.'),
      ),
    );
    const { user } = renderApp('/jobs/new');
    await chooseAccount(user);
    await tick(user, /acme\/shop/);

    expect(await screen.findByText('The GitHub App is not installed on this repository.')).toBeInTheDocument();
    expect(submitButton()).toBeDisabled();
  });

  it('flags repositories without pull access in the list', async () => {
    server.use(
      http.get('/api/github/repositories', () =>
        HttpResponse.json({
          ...f.repositories,
          items: [{ ...f.repository, permissions: { pull: false, push: false, admin: false } }],
        }),
      ),
    );
    const { user } = renderApp('/jobs/new');
    await chooseAccount(user);
    expect(await screen.findByText('no read access')).toBeInTheDocument();
  });

  it('prefills from query parameters', async () => {
    renderApp('/jobs/new?owner=acme&repo=shop&branch=develop&model=claude-haiku-4.5');
    expect(await screen.findByText('Selected (1)')).toBeInTheDocument();
    const branch = (await screen.findByLabelText('Base branch for acme/shop')) as HTMLSelectElement;
    await waitFor(() => expect(branch.value).toBe('develop'));
    expect(await screen.findByRole('checkbox', { name: /acme\/shop/ })).toBeChecked();
    expect((screen.getByLabelText('4. Model') as HTMLSelectElement).value).toBe('claude-haiku-4.5');
  });
});
