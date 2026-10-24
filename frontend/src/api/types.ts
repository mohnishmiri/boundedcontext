// Mirrors docs/api.md. Keep in sync with the backend contract; field names are the wire names.

export type ErrorCode =
  | 'unauthenticated'
  | 'csrf_rejected'
  | 'forbidden'
  | 'not_found'
  | 'validation_failed'
  | 'duplicate_job'
  | 'conflict'
  | 'rate_limited'
  | 'payload_too_large'
  | 'github_unavailable'
  | 'github_auth_expired'
  | 'github_rate_limited'
  | 'repository_access_denied'
  | 'app_not_installed'
  | 'model_unavailable'
  | 'model_locked_by_agent'
  | 'generator_not_allowed'
  | 'generator_invalid'
  | 'copilot_unauthorized'
  | 'agent_failed'
  | 'agent_timeout'
  | 'agent_unavailable'
  | 'output_missing'
  | 'output_empty'
  | 'output_invalid'
  | 'output_too_large'
  | 'repository_too_large'
  | 'base_branch_changed'
  | 'branch_conflict'
  | 'cancelled'
  | 'internal';

export type ApiErrorBody = {
  error: {
    code: ErrorCode;
    message: string;
    retryable: boolean;
    details?: Record<string, unknown>;
    request_id: string;
  };
};

export type JobStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled';
export type GenerationStage =
  | 'queued'
  | 'preparing'
  | 'fetching_repository'
  | 'running_agent'
  | 'validating_output'
  | 'completed';
export type PublishStatus = 'not_started' | 'queued' | 'running' | 'succeeded' | 'failed';
export type PublishStage =
  | 'queued'
  | 'checking_base'
  | 'creating_commit'
  | 'creating_branch'
  | 'creating_pull_request'
  | 'completed';
export type StepState = 'pending' | 'active' | 'done' | 'failed' | 'skipped';

export type Step = {
  key: string;
  label: string;
  state: StepState;
  started_at: string | null;
  finished_at: string | null;
};

export type JobError = { code: ErrorCode; message: string; retryable: boolean };

export type User = { id: string; login: string; name: string | null; avatar_url: string | null };

export type JobSummary = {
  id: string;
  short_id: string;
  owner: string;
  repo: string;
  base_branch: string;
  model: string;
  status: JobStatus;
  stage: GenerationStage;
  publish_status: PublishStatus;
  pr_url: string | null;
  created_at: string;
  finished_at: string | null;
  requested_by: string;
  batch_id: string | null;
};

export type OutputChangeType = 'added' | 'modified';
export type AnyChangeType = 'added' | 'modified' | 'deleted';

export type JobOutput = {
  primary_path: string;
  files: { path: string; change_type: OutputChangeType; size_bytes: number }[];
  unexpected_changes: { path: string; change_type: AnyChangeType }[];
};

export type JobPublish = {
  status: PublishStatus;
  stage: PublishStage | null;
  steps: Step[];
  branch: string | null;
  commit_sha: string | null;
  commit_url: string | null;
  pr_number: number | null;
  pr_url: string | null;
  error: JobError | null;
  updated_at: string | null;
};

export type GeneratorSource = 'builtin' | 'curated' | 'user';

/** The generator a job ran with, as recorded on the job (may point at a since-removed generator). */
export type JobGenerator = {
  id: string | null;
  name: string;
  source: GeneratorSource;
  owner: string | null;
  repo: string | null;
  branch: string | null;
  path: string | null;
  sha: string | null;
};

export type Job = JobSummary & {
  repository_url: string;
  base_sha: string | null;
  agent: { id: string; version: string; placeholder: boolean };
  generator: JobGenerator | null;
  inputs: Record<string, string>;
  steps: Step[];
  error: JobError | null;
  cancel_requested: boolean;
  can_cancel: boolean;
  can_publish: boolean;
  can_retry: boolean;
  started_at: string | null;
  updated_at: string;
  duration_ms: number | null;
  output: JobOutput | null;
  publish: JobPublish;
};

export type LogLevel = 'debug' | 'info' | 'warn' | 'error';

export type LogEntry = { seq: number; ts: string; level: LogLevel; stage: string; message: string };

export type Page<T> = { items: T[]; page: number; per_page: number; has_more: boolean; total?: number };

export type Organization = {
  installation_id: number;
  login: string;
  type: 'Organization' | 'User';
  avatar_url: string | null;
};

export type OrganizationList = { items: Organization[]; install_url: string };

export type Repository = {
  id: number;
  owner: string;
  name: string;
  full_name: string;
  private: boolean;
  default_branch: string;
  html_url: string;
  description: string | null;
  permissions: { pull: boolean; push: boolean; admin: boolean };
  updated_at: string | null;
};

export type RepositoryDetail = Repository & {
  can_generate: boolean;
  can_publish: boolean;
  reason: string | null;
};

export type Branch = { name: string; commit_sha: string; protected: boolean };

export type BranchPage = Page<Branch> & { default_branch: string };

export type AgentInputKind = 'text' | 'select' | 'path';

export type AgentInput = {
  name: string;
  label: string;
  help: string | null;
  required: boolean;
  kind: AgentInputKind;
  choices: string[] | null;
  default: string | null;
  max_length: number;
};

export type AgentInfo = {
  id: string;
  name: string;
  description: string;
  version: string;
  placeholder: boolean;
  output_paths: string[];
  inputs: AgentInput[];
};

export type Generator = {
  id: string;
  name: string;
  description: string | null;
  source: GeneratorSource;
  repository: { owner: string; repo: string; branch: string | null; path: string } | null;
  removable: boolean;
};

export type GeneratorList = { items: Generator[]; default_id: string };

export type CreateGeneratorRequest = {
  owner: string;
  repo: string;
  branch?: string;
  path?: string;
  name?: string;
};

/** `GET /api/generators/{id}/agent`: the agent info plus the generator's model pin and resolved revision. */
export type GeneratorAgentInfo = AgentInfo & {
  locked_model: string | null;
  lock_reason: string | null;
  resolved: { branch: string | null; sha: string | null };
};

export type Model = {
  id: string;
  name: string;
  vendor: string;
  description: string | null;
  multiplier: number | null;
  is_default: boolean;
};

export type ModelList = {
  items: Model[];
  default_model: string;
  /** `copilot`: the models the signed-in user can use; `catalog`: a static fallback list. */
  source: 'copilot' | 'catalog';
  warning: string | null;
  entitlement_note: string;
};

export type CreateJobRequest = {
  owner: string;
  repo: string;
  base_branch: string;
  model: string;
  inputs?: Record<string, string>;
  generator_id?: string;
};

export type BatchRepository = { owner: string; repo: string; base_branch: string };

export type CreateBatchRequest = {
  generator_id: string;
  model: string;
  inputs: Record<string, string>;
  repositories: BatchRepository[];
};

export type BatchFailure = BatchRepository & {
  error: { code: ErrorCode; message: string; details?: unknown };
};

export type BatchResponse = { batch_id: string; jobs: JobSummary[]; failures: BatchFailure[] };

export type DiffFile = {
  path: string;
  change_type: OutputChangeType;
  base_content: string | null;
  new_content: string;
  unified_diff: string;
  size_bytes: number;
  sha256: string;
};

export type JobDiff = {
  base_sha: string;
  files: DiffFile[];
  unexpected_changes: { path: string; change_type: AnyChangeType }[];
};

export type LogPage = { items: LogEntry[]; next_after: number };
