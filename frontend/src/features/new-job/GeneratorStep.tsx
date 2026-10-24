import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState, type KeyboardEvent } from 'react';

import * as api from '../../api/client';
import { isApiError } from '../../api/client';
import { queryKeys, useGenerators } from '../../api/queries';
import type { CreateGeneratorRequest, Generator } from '../../api/types';
import { Alert } from '../../components/Alert';
import { Button } from '../../components/Button';
import { ErrorPanel } from '../../components/ErrorPanel';
import { Spinner } from '../../components/Spinner';
import { inputClass } from '../../lib/styles';

const SOURCE_LABELS: Record<Generator['source'], string> = {
  builtin: 'built-in',
  curated: 'curated',
  user: 'added by you',
};

const emptyForm = { owner: '', repo: '', branch: '', path: '', name: '' };
type FormState = typeof emptyForm;

function repositoryLabel(g: Generator): string | null {
  if (!g.repository) return null;
  const { owner, repo, branch, path } = g.repository;
  return `${owner}/${repo}${branch ? `@${branch}` : ''}${path ? `:${path}` : ''}`;
}

function AddGeneratorForm({ onAdded, onCancel }: { onAdded: (g: Generator) => void; onCancel: () => void }) {
  const qc = useQueryClient();
  const [form, setForm] = useState<FormState>(emptyForm);
  const [clientErrors, setClientErrors] = useState<Record<string, string>>({});

  const add = useMutation({
    mutationFn: (body: CreateGeneratorRequest) => api.createGenerator(body),
    onSuccess: async (generator) => {
      await qc.invalidateQueries({ queryKey: queryKeys.generators });
      onAdded(generator);
    },
  });

  const set = (key: keyof FormState) => (value: string) => {
    setForm((prev) => ({ ...prev, [key]: value }));
    if (clientErrors[key]) setClientErrors((prev) => ({ ...prev, [key]: '' }));
  };

  const submit = () => {
    const errors: Record<string, string> = {};
    if (!form.owner.trim()) errors.owner = 'This field is required';
    if (!form.repo.trim()) errors.repo = 'This field is required';
    setClientErrors(errors);
    if (Object.keys(errors).length > 0) return;
    const body: CreateGeneratorRequest = { owner: form.owner.trim(), repo: form.repo.trim() };
    if (form.branch.trim()) body.branch = form.branch.trim();
    if (form.path.trim()) body.path = form.path.trim();
    if (form.name.trim()) body.name = form.name.trim();
    add.mutate(body);
  };

  // This form lives inside the page form; Enter must add the generator, not start a generation.
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      submit();
    }
  };

  const err = add.error;
  const serverFields = isApiError(err) && err.code === 'validation_failed' ? err.fieldErrors : {};
  const hasFieldErrors = Object.keys(serverFields).length > 0;

  const fields: { key: keyof FormState; label: string; placeholder?: string; required?: boolean }[] = [
    { key: 'owner', label: 'Owner', placeholder: 'e.g. my-org', required: true },
    { key: 'repo', label: 'Repository', placeholder: 'e.g. bc-generator', required: true },
    { key: 'branch', label: 'Branch (optional)', placeholder: 'default branch' },
    { key: 'path', label: 'Path in repository (optional)', placeholder: 'e.g. agents/bounded-context' },
    { key: 'name', label: 'Display name (optional)' },
  ];

  return (
    <div
      role="group"
      aria-label="Add a generator repository"
      className="space-y-3 rounded-md border border-gray-200 p-4 dark:border-gray-700"
    >
      <p className="text-sm text-gray-600 dark:text-gray-300">
        Point the portal at a repository that contains a generator bundle. The repository must be on the operator’s
        allowlist and you need read access to it.
      </p>
      <div className="grid gap-3 sm:grid-cols-2">
        {fields.map(({ key, label, placeholder, required }) => {
          const id = `generator-${key}`;
          const error = clientErrors[key] || serverFields[key];
          return (
            <div key={key} className="space-y-1">
              <label htmlFor={id} className="block text-sm font-medium">
                {label}
                {required && (
                  <span className="text-red-600" aria-hidden="true">
                    {' '}
                    *
                  </span>
                )}
              </label>
              <input
                id={id}
                type="text"
                className={inputClass}
                value={form[key]}
                placeholder={placeholder}
                autoComplete="off"
                spellCheck={false}
                required={required}
                aria-invalid={error ? true : undefined}
                aria-describedby={error ? `${id}-error` : undefined}
                onChange={(e) => set(key)(e.target.value)}
                onKeyDown={onKeyDown}
              />
              {error && (
                <p id={`${id}-error`} className="text-sm text-red-600 dark:text-red-400" role="alert">
                  {error}
                </p>
              )}
            </div>
          );
        })}
      </div>
      {err && !hasFieldErrors && <ErrorPanel error={err} title="Could not add the generator" />}
      {err && hasFieldErrors && (
        <Alert tone="error" role="alert">
          Some fields need attention.
        </Alert>
      )}
      <div className="flex gap-2">
        <Button onClick={submit} loading={add.isPending}>
          Add generator
        </Button>
        <Button variant="ghost" onClick={onCancel} disabled={add.isPending}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

export function GeneratorStep({
  selectedId,
  onSelect,
  error,
}: {
  selectedId: string | null;
  onSelect: (id: string) => void;
  error?: string;
}) {
  const qc = useQueryClient();
  const generators = useGenerators();
  const [adding, setAdding] = useState(false);

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteGenerator(id),
    onSuccess: async (_data, id) => {
      qc.removeQueries({ queryKey: queryKeys.generatorAgent(id) });
      await qc.invalidateQueries({ queryKey: queryKeys.generators });
    },
  });

  return (
    <fieldset className="space-y-3">
      <legend className="text-base font-semibold">1. Generator</legend>
      <p className="text-sm text-gray-600 dark:text-gray-300">
        The generator is the agent that writes the documentation.
      </p>
      {generators.isPending && <Spinner label="Loading generators…" />}
      {generators.isError && <ErrorPanel error={generators.error} title="Could not load generators" />}
      {generators.data && (
        <ul className="divide-y divide-gray-100 rounded-md border border-gray-200 dark:divide-gray-800 dark:border-gray-700">
          {generators.data.items.map((g) => {
            const checked = selectedId === g.id;
            const where = repositoryLabel(g);
            return (
              <li key={g.id} className="flex items-start gap-2 pr-3">
                <label
                  className={`flex min-w-0 flex-1 cursor-pointer items-start gap-3 px-3 py-2 text-sm focus-within:bg-indigo-50 dark:focus-within:bg-indigo-950/40 ${
                    checked ? 'bg-indigo-50 dark:bg-indigo-950/40' : 'hover:bg-gray-50 dark:hover:bg-gray-800'
                  }`}
                >
                  <input
                    type="radio"
                    name="generator"
                    value={g.id}
                    checked={checked}
                    onChange={() => onSelect(g.id)}
                    className="mt-1 h-4 w-4 accent-indigo-600"
                  />
                  <span className="min-w-0 flex-1">
                    <span className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{g.name}</span>
                      <span className="rounded bg-gray-100 px-1.5 text-xs text-gray-600 dark:bg-gray-800 dark:text-gray-300">
                        {SOURCE_LABELS[g.source]}
                      </span>
                      {g.id === generators.data.default_id && (
                        <span className="rounded bg-indigo-100 px-1.5 text-xs text-indigo-800 dark:bg-indigo-950 dark:text-indigo-200">
                          default
                        </span>
                      )}
                    </span>
                    {g.description && (
                      <span className="block text-xs text-gray-600 dark:text-gray-300">{g.description}</span>
                    )}
                    {where && <span className="block font-mono text-xs text-gray-500 dark:text-gray-400">{where}</span>}
                  </span>
                </label>
                {g.removable && (
                  <Button
                    variant="ghost"
                    className="mt-1"
                    aria-label={`Remove generator ${g.name}`}
                    disabled={remove.isPending}
                    onClick={() => remove.mutate(g.id)}
                  >
                    Remove
                  </Button>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {remove.isError && <ErrorPanel error={remove.error} title="Could not remove the generator" />}
      {error && (
        <p className="text-sm text-red-600 dark:text-red-400" role="alert">
          {error}
        </p>
      )}
      {adding ? (
        <AddGeneratorForm
          onCancel={() => setAdding(false)}
          onAdded={(g) => {
            setAdding(false);
            onSelect(g.id);
          }}
        />
      ) : (
        <Button variant="secondary" onClick={() => setAdding(true)}>
          Add a generator repository
        </Button>
      )}
    </fieldset>
  );
}
