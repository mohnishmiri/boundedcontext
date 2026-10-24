import type {
  AgentInfo,
  BranchPage,
  Job,
  JobDiff,
  ModelList,
  OrganizationList,
  Page,
  Repository,
  RepositoryDetail,
  Step,
  User,
} from '../api/types';

export const user: User = { id: 'u1', login: 'octocat', name: 'Mona', avatar_url: null };

export const organizations: OrganizationList = {
  items: [
    { installation_id: 1, login: 'acme', type: 'Organization', avatar_url: null },
    { installation_id: 2, login: 'octocat', type: 'User', avatar_url: null },
  ],
  install_url: 'https://github.com/apps/bounded-context-portal/installations/new',
};

export const repository: Repository = {
  id: 10,
  owner: 'acme',
  name: 'shop',
  full_name: 'acme/shop',
  private: true,
  default_branch: 'main',
  html_url: 'https://github.com/acme/shop',
  description: 'Online shop',
  permissions: { pull: true, push: true, admin: false },
  updated_at: '2026-10-01T10:00:00Z',
};

export const repositories: Page<Repository> = {
  items: [repository, { ...repository, id: 11, name: 'billing', full_name: 'acme/billing', description: null }],
  page: 1,
  per_page: 30,
  has_more: false,
};

export const repositoryDetail: RepositoryDetail = { ...repository, can_generate: true, can_publish: true, reason: null };

export const branches: BranchPage = {
  items: [
    { name: 'main', commit_sha: 'a1b2c3d4e5f6a7b8c9d0', protected: true },
    { name: 'develop', commit_sha: 'b1b2c3d4e5f6a7b8c9d0', protected: false },
  ],
  page: 1,
  per_page: 50,
  has_more: false,
  default_branch: 'main',
};

export const agent: AgentInfo = {
  id: 'bounded-context',
  name: 'Bounded Context Generator',
  description: 'Writes a bounded-context document.',
  version: 'sha256:abc123',
  placeholder: true,
  output_paths: ['docs/bounded-context.md'],
  inputs: [
    {
      name: 'scope_path',
      label: 'Subdirectory to analyse',
      help: 'Leave empty to analyse the whole repository.',
      required: false,
      kind: 'path',
      choices: null,
      default: '',
      max_length: 200,
    },
  ],
};

export const models: ModelList = {
  items: [
    { id: 'auto', name: 'Auto', vendor: 'GitHub', description: 'Copilot chooses.', multiplier: null, is_default: true },
    { id: 'claude-haiku-4.5', name: 'Claude Haiku 4.5', vendor: 'Anthropic', description: null, multiplier: 0.33, is_default: false },
  ],
  default_model: 'auto',
  locked_model: null,
  lock_reason: null,
  entitlement_note: 'Availability is enforced by GitHub Copilot for your account at run time.',
};

export const JOB_ID = '0b9c2f8e-1111-4c4c-9d9d-123456789abc';

const generationSteps = (states: Step['state'][]): Step[] =>
  [
    ['queued', 'Queued'],
    ['preparing', 'Checking access and resolving branch'],
    ['fetching_repository', 'Fetching repository snapshot'],
    ['running_agent', 'Running bounded-context agent'],
    ['validating_output', 'Validating generated documentation'],
  ].map(([key, label], i) => ({
    key,
    label,
    state: states[i] ?? 'pending',
    started_at: states[i] && states[i] !== 'pending' ? '2026-10-24T10:00:00Z' : null,
    finished_at: states[i] === 'done' || states[i] === 'failed' ? '2026-10-24T10:01:00Z' : null,
  }));

export function makeJob(overrides: Partial<Job> = {}): Job {
  return {
    id: JOB_ID,
    short_id: JOB_ID.slice(0, 8),
    owner: 'acme',
    repo: 'shop',
    base_branch: 'main',
    model: 'auto',
    status: 'queued',
    stage: 'queued',
    publish_status: 'not_started',
    pr_url: null,
    created_at: '2026-10-24T10:00:00Z',
    finished_at: null,
    requested_by: 'octocat',
    repository_url: 'https://github.com/acme/shop',
    base_sha: null,
    agent: { id: 'bounded-context', version: 'sha256:abc123', placeholder: true },
    inputs: {},
    steps: generationSteps(['active']),
    error: null,
    cancel_requested: false,
    can_cancel: true,
    can_publish: false,
    can_retry: false,
    started_at: null,
    updated_at: '2026-10-24T10:00:00Z',
    duration_ms: null,
    output: null,
    publish: {
      status: 'not_started',
      stage: null,
      steps: [],
      branch: null,
      commit_sha: null,
      commit_url: null,
      pr_number: null,
      pr_url: null,
      error: null,
      updated_at: null,
    },
    ...overrides,
  };
}

export function runningJob(overrides: Partial<Job> = {}): Job {
  return makeJob({
    status: 'running',
    stage: 'running_agent',
    base_sha: 'a1b2c3d4e5f6a7b8c9d0',
    started_at: '2026-10-24T10:00:05Z',
    steps: generationSteps(['done', 'done', 'done', 'active']),
    ...overrides,
  });
}

export function succeededJob(overrides: Partial<Job> = {}): Job {
  return makeJob({
    status: 'succeeded',
    stage: 'completed',
    base_sha: 'a1b2c3d4e5f6a7b8c9d0',
    started_at: '2026-10-24T10:00:05Z',
    finished_at: '2026-10-24T10:05:05Z',
    duration_ms: 300_000,
    steps: generationSteps(['done', 'done', 'done', 'done', 'done']),
    can_cancel: false,
    can_publish: true,
    output: {
      primary_path: 'docs/bounded-context.md',
      files: [{ path: 'docs/bounded-context.md', change_type: 'added', size_bytes: 120 }],
      unexpected_changes: [{ path: 'README.md', change_type: 'modified' }],
    },
    ...overrides,
  });
}

export function failedJob(overrides: Partial<Job> = {}): Job {
  return makeJob({
    status: 'failed',
    stage: 'running_agent',
    base_sha: 'a1b2c3d4e5f6a7b8c9d0',
    started_at: '2026-10-24T10:00:05Z',
    finished_at: '2026-10-24T10:20:05Z',
    duration_ms: 1_200_000,
    steps: generationSteps(['done', 'done', 'done', 'failed', 'skipped']),
    can_cancel: false,
    can_retry: true,
    error: { code: 'agent_timeout', message: 'The agent did not finish within 20 minutes.', retryable: true },
    ...overrides,
  });
}

export const diff: JobDiff = {
  base_sha: 'a1b2c3d4e5f6a7b8c9d0',
  files: [
    {
      path: 'docs/bounded-context.md',
      change_type: 'added',
      base_content: null,
      new_content: '# Bounded Contexts — shop\n\n## Summary\n\nOrders and billing.\n',
      unified_diff:
        '--- /dev/null\n+++ b/docs/bounded-context.md\n@@ -0,0 +1,5 @@\n+# Bounded Contexts — shop\n+\n+## Summary\n+\n+Orders and billing.\n',
      size_bytes: 58,
      sha256: 'deadbeef',
    },
  ],
  unexpected_changes: [{ path: 'README.md', change_type: 'modified' }],
};
