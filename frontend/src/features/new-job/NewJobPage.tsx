import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useMemo, useState, type FormEvent } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';

import * as api from '../../api/client';
import { isApiError } from '../../api/client';
import { useGenerators, useGeneratorAgent, useModels, useRepositoryDetails } from '../../api/queries';
import type { BatchResponse, CreateBatchRequest, Repository } from '../../api/types';
import { Alert } from '../../components/Alert';
import { Button } from '../../components/Button';
import { ErrorPanel } from '../../components/ErrorPanel';
import { PlaceholderBanner } from '../../components/PlaceholderBanner';
import { Spinner } from '../../components/Spinner';
import { cardClass, linkClass } from '../../lib/styles';
import { storeBatchFailures } from '../batch/batchFailures';
import { AgentInputFields } from './AgentInputFields';
import { GeneratorStep } from './GeneratorStep';
import { ModelField } from './ModelField';
import { OrganizationStep } from './OrganizationStep';
import { RepositoryStep } from './RepositoryStep';
import { MAX_BATCH_REPOS, repoKey, type SelectedRepo, type Selection } from './selection';
import { SelectedRepositories } from './SelectedRepositories';
import { inputFieldError, validateInput } from './validation';

function initialSelection(owner: string | null, repo: string | null, branch: string | null): Selection {
  if (!owner || !repo) return {};
  return { [repoKey(owner, repo)]: { owner, repo, defaultBranch: null, branch } };
}

export function NewJobPage() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [params] = useSearchParams();

  // Optional prefill (e.g. "generate again" links): ?owner=&repo=&branch=&model=&generator=
  const [owner, setOwner] = useState<string | null>(params.get('owner'));
  const [generatorChoice, setGeneratorChoice] = useState<string | null>(params.get('generator'));
  const [selection, setSelection] = useState<Selection>(() =>
    initialSelection(params.get('owner'), params.get('repo'), params.get('branch')),
  );
  const [model, setModel] = useState<string | null>(params.get('model'));
  const [inputOverrides, setInputOverrides] = useState<Record<string, string>>({});
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const generators = useGenerators();
  const generatorId = useMemo(() => {
    const list = generators.data;
    if (!list) return null;
    if (generatorChoice && list.items.some((g) => g.id === generatorChoice)) return generatorChoice;
    return list.default_id;
  }, [generators.data, generatorChoice]);
  const agent = useGeneratorAgent(generatorId);
  const models = useModels();

  const entries = Object.values(selection);
  const details = useRepositoryDetails(entries);
  const branchOf = (entry: SelectedRepo): string | null => {
    const i = entries.indexOf(entry);
    return entry.branch ?? entry.defaultBranch ?? details[i]?.data?.default_branch ?? null;
  };

  const lockedModel = agent.data?.locked_model ?? null;
  const effectiveModel = useMemo(() => {
    const m = models.data;
    if (!m) return null;
    if (lockedModel) return lockedModel;
    if (model && m.items.some((x) => x.id === model)) return model;
    return m.default_model;
  }, [models.data, model, lockedModel]);
  const agentInputs = useMemo(() => agent.data?.inputs ?? [], [agent.data]);
  const inputValues = useMemo(() => {
    const values: Record<string, string> = {};
    for (const input of agentInputs) values[input.name] = inputOverrides[input.name] ?? input.default ?? '';
    return values;
  }, [agentInputs, inputOverrides]);

  const create = useMutation({
    mutationFn: (body: CreateBatchRequest) => api.createBatch(body),
    onSuccess: (res: BatchResponse) => {
      qc.invalidateQueries({ queryKey: ['jobs', 'list'] });
      if (res.jobs.length === 1 && res.failures.length === 0) {
        navigate(`/jobs/${encodeURIComponent(res.jobs[0].id)}`);
        return;
      }
      storeBatchFailures(res.batch_id, res.failures);
      navigate(`/batches/${encodeURIComponent(res.batch_id)}`, { state: { failures: res.failures } });
    },
    onError: (err) => {
      if (isApiError(err) && err.code === 'validation_failed') setFieldErrors(err.fieldErrors);
    },
  });

  const inputErrors: Record<string, string | undefined> = {};
  for (const input of agentInputs) inputErrors[input.name] = inputFieldError(fieldErrors, input.name);

  const reposReady =
    entries.length > 0 &&
    entries.every((entry, i) => branchOf(entry) !== null && details[i]?.data?.can_generate === true);
  const canSubmit =
    Boolean(generatorId && agent.data && effectiveModel) && reposReady && !create.isPending;

  const resetFeedback = () => {
    create.reset();
    setFieldErrors({});
  };

  const toggleRepo = (r: Repository, selected: boolean) => {
    resetFeedback();
    const key = repoKey(r.owner, r.name);
    setSelection((prev) => {
      if (!selected) {
        const next = { ...prev };
        delete next[key];
        return next;
      }
      if (key in prev || Object.keys(prev).length >= MAX_BATCH_REPOS) return prev;
      return { ...prev, [key]: { owner: r.owner, repo: r.name, defaultBranch: r.default_branch, branch: null } };
    });
  };

  const removeRepo = (key: string) => {
    resetFeedback();
    setSelection((prev) => {
      const next = { ...prev };
      delete next[key];
      return next;
    });
  };

  const setBranch = (key: string, branch: string) => {
    resetFeedback();
    setSelection((prev) => (prev[key] ? { ...prev, [key]: { ...prev[key], branch } } : prev));
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit || !generatorId || !effectiveModel) return;
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
    const repositories = entries.flatMap((entry) => {
      const base_branch = branchOf(entry);
      return base_branch ? [{ owner: entry.owner, repo: entry.repo, base_branch }] : [];
    });
    create.mutate({ generator_id: generatorId, model: effectiveModel, inputs, repositories });
  };

  const err = create.error;
  const hasFieldErrors = isApiError(err) && err.code === 'validation_failed' && Object.keys(err.fieldErrors).length > 0;
  const repositoriesError =
    fieldErrors.repositories ??
    Object.entries(fieldErrors).find(([k]) => k.startsWith('repositories.'))?.[1] ??
    fieldErrors.owner;

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold">New generation</h1>
        <p className="text-sm text-gray-600 dark:text-gray-300">
          Run {agent.data ? <strong>{agent.data.name}</strong> : 'a generator'} against one or more repositories. You can
          review each result before anything is written to GitHub.
        </p>
      </div>

      <form onSubmit={onSubmit} noValidate className={`${cardClass} space-y-8`} aria-label="New generation">
        <GeneratorStep
          selectedId={generatorId}
          error={fieldErrors.generator_id}
          onSelect={(id) => {
            if (id === generatorId) return;
            setGeneratorChoice(id);
            setInputOverrides({});
            resetFeedback();
          }}
        />

        {agent.isPending && generatorId && <Spinner label="Loading generator configuration…" />}
        {agent.isError && <ErrorPanel error={agent.error} title="Could not load the generator configuration" />}
        {agent.data?.placeholder && <PlaceholderBanner name={agent.data.name} />}

        <OrganizationStep
          owner={owner}
          onChange={(next) => {
            if (next === owner) return;
            setOwner(next);
            resetFeedback();
          }}
        />

        {owner && (
          <RepositoryStep
            key={owner}
            owner={owner}
            selection={selection}
            error={repositoriesError}
            onToggle={toggleRepo}
          />
        )}

        <SelectedRepositories selection={selection} branchOf={branchOf} onBranchChange={setBranch} onRemove={removeRepo} />

        {models.isPending && <Spinner label="Loading models…" />}
        {models.isError && <ErrorPanel error={models.error} title="Could not load models" />}
        {models.data && effectiveModel && (
          <ModelField
            models={models.data}
            value={effectiveModel}
            onChange={setModel}
            error={fieldErrors.model}
            lockedModel={lockedModel}
            lockReason={agent.data?.lock_reason}
          />
        )}

        <AgentInputFields
          inputs={agentInputs}
          values={inputValues}
          errors={inputErrors}
          onChange={(name, value) => setInputOverrides((prev) => ({ ...prev, [name]: value }))}
        />

        {agent.data && agent.data.output_paths.length > 0 && (
          <p className="text-xs text-gray-500 dark:text-gray-400">
            The generator writes{' '}
            {agent.data.output_paths.map((p, i) => (
              <span key={p}>
                {i > 0 && ', '}
                <code>{p}</code>
              </span>
            ))}
            . Only these files can ever be committed; nothing is pushed until you choose to open a pull request.
          </p>
        )}

        {err && !hasFieldErrors && <ErrorPanel error={err} title="Could not start the generation" />}
        {hasFieldErrors && (
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
