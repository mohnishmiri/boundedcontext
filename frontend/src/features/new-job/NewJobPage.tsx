import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useMemo, useState, type FormEvent } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';

import * as api from '../../api/client';
import { isApiError } from '../../api/client';
import { queryKeys, useAgent, useModels, useRepository } from '../../api/queries';
import type { CreateJobRequest } from '../../api/types';
import { Alert } from '../../components/Alert';
import { Button } from '../../components/Button';
import { ErrorPanel } from '../../components/ErrorPanel';
import { Spinner } from '../../components/Spinner';
import { cardClass, linkClass } from '../../lib/styles';
import { AgentInputFields } from './AgentInputFields';
import { BranchStep } from './BranchStep';
import { ModelField } from './ModelField';
import { OrganizationStep } from './OrganizationStep';
import { RepositoryStep } from './RepositoryStep';
import { inputFieldError, validateInput } from './validation';

const JOB_ID_RE = /^[0-9a-fA-F-]{8,64}$/;

export function NewJobPage() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [params] = useSearchParams();

  // Optional prefill (e.g. "generate again" links): ?owner=&repo=&branch=&model=
  const [owner, setOwner] = useState<string | null>(params.get('owner'));
  const [repo, setRepo] = useState<string | null>(params.get('repo'));
  const [branch, setBranch] = useState<string | null>(params.get('branch'));
  const [model, setModel] = useState<string | null>(params.get('model'));
  const [inputOverrides, setInputOverrides] = useState<Record<string, string>>({});
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const agent = useAgent();
  const models = useModels();
  const repoDetail = useRepository(owner, repo);

  const effectiveBranch = branch ?? repoDetail.data?.default_branch ?? null;
  const effectiveModel = useMemo(() => {
    const m = models.data;
    if (!m) return null;
    if (m.locked_model) return m.locked_model;
    if (model && m.items.some((x) => x.id === model)) return model;
    return m.default_model;
  }, [models.data, model]);
  const agentInputs = useMemo(() => agent.data?.inputs ?? [], [agent.data]);
  const inputValues = useMemo(() => {
    const values: Record<string, string> = {};
    for (const input of agentInputs) values[input.name] = inputOverrides[input.name] ?? input.default ?? '';
    return values;
  }, [agentInputs, inputOverrides]);

  const create = useMutation({
    mutationFn: (body: CreateJobRequest) => api.createJob(body),
    onSuccess: (job) => {
      qc.setQueryData(queryKeys.job(job.id), job);
      qc.invalidateQueries({ queryKey: ['jobs', 'list'] });
      navigate(`/jobs/${encodeURIComponent(job.id)}`);
    },
    onError: (err) => {
      if (isApiError(err) && err.code === 'validation_failed') setFieldErrors(err.fieldErrors);
    },
  });

  const inputErrors: Record<string, string | undefined> = {};
  for (const input of agentInputs) inputErrors[input.name] = inputFieldError(fieldErrors, input.name);

  const canSubmit =
    Boolean(owner && repo && effectiveBranch && effectiveModel) &&
    repoDetail.data?.can_generate === true &&
    !create.isPending;

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!owner || !repo || !effectiveBranch || !effectiveModel) return;
    const errors: Record<string, string> = {};
    for (const input of agentInputs) {
      const msg = validateInput(input, inputValues[input.name] ?? '');
      if (msg) errors[input.name] = msg;
    }
    setFieldErrors(errors);
    if (Object.keys(errors).length > 0) return;
    const inputs: Record<string, string> = {};
    for (const input of agentInputs) {
      const v = inputValues[input.name] ?? '';
      if (v !== '') inputs[input.name] = v;
    }
    create.mutate({
      owner,
      repo,
      base_branch: effectiveBranch,
      model: effectiveModel,
      ...(Object.keys(inputs).length > 0 ? { inputs } : {}),
    });
  };

  const err = create.error;
  const duplicateJobId =
    isApiError(err) && err.code === 'duplicate_job' && typeof err.details?.job_id === 'string' && JOB_ID_RE.test(err.details.job_id)
      ? err.details.job_id
      : null;

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold">New generation</h1>
        <p className="text-sm text-gray-600 dark:text-gray-300">
          Run {agent.data ? <strong>{agent.data.name}</strong> : 'the bounded-context agent'} against a branch. You can
          review the result before anything is written to GitHub.
        </p>
      </div>

      <form onSubmit={onSubmit} noValidate className={`${cardClass} space-y-8`} aria-label="New generation">
        <OrganizationStep
          owner={owner}
          error={fieldErrors.owner}
          onChange={(next) => {
            if (next === owner) return;
            setOwner(next);
            setRepo(null);
            setBranch(null);
            create.reset();
          }}
        />

        {owner && (
          <RepositoryStep
            key={owner}
            owner={owner}
            repo={repo}
            error={fieldErrors.repo}
            onChange={(next) => {
              setRepo(next);
              setBranch(null);
              create.reset();
            }}
          />
        )}

        {owner && repo && (
          <BranchStep
            key={`${owner}/${repo}`}
            owner={owner}
            repo={repo}
            branch={effectiveBranch}
            defaultBranch={repoDetail.data?.default_branch ?? null}
            error={fieldErrors.base_branch}
            onChange={(next) => {
              setBranch(next);
              create.reset();
            }}
          />
        )}

        {models.isPending && <Spinner label="Loading models…" />}
        {models.isError && <ErrorPanel error={models.error} title="Could not load models" />}
        {models.data && effectiveModel && (
          <ModelField models={models.data} value={effectiveModel} onChange={setModel} error={fieldErrors.model} />
        )}

        {agent.isError && <ErrorPanel error={agent.error} title="Could not load the agent configuration" />}
        <AgentInputFields
          inputs={agentInputs}
          values={inputValues}
          errors={inputErrors}
          onChange={(name, value) => setInputOverrides((prev) => ({ ...prev, [name]: value }))}
        />

        {agent.data && agent.data.output_paths.length > 0 && (
          <p className="text-xs text-gray-500 dark:text-gray-400">
            The agent writes{' '}
            {agent.data.output_paths.map((p, i) => (
              <span key={p}>
                {i > 0 && ', '}
                <code>{p}</code>
              </span>
            ))}
            . Only these files can ever be committed; nothing is pushed until you choose to open a pull request.
          </p>
        )}

        {duplicateJobId ? (
          <Alert tone="warning" title="A matching generation is already running" role="alert">
            <p>{err?.message}</p>
            <Link to={`/jobs/${encodeURIComponent(duplicateJobId)}`} className={linkClass}>
              Open the existing job
            </Link>
          </Alert>
        ) : (
          err &&
          !(isApiError(err) && err.code === 'validation_failed' && Object.keys(err.fieldErrors).length > 0) && (
            <ErrorPanel error={err} title="Could not start the generation" />
          )
        )}
        {isApiError(err) && err.code === 'validation_failed' && Object.keys(err.fieldErrors).length > 0 && (
          <Alert tone="error" role="alert">
            Some fields need attention.
          </Alert>
        )}

        <div className="flex items-center gap-3">
          <Button type="submit" disabled={!canSubmit} loading={create.isPending}>
            Generate documentation
          </Button>
          <Link to="/" className={linkClass}>
            Cancel
          </Link>
        </div>
      </form>
    </div>
  );
}
