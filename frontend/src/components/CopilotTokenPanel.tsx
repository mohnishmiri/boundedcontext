import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';

import * as api from '../api/client';
import { queryKeys } from '../api/queries';
import { inputClass, linkClass } from '../lib/styles';
import { Alert } from './Alert';
import { Button } from './Button';

const NEW_TOKEN_URL =
  'https://github.com/settings/personal-access-tokens/new?name=Bounded%20Context%20Portal%20Copilot&description=Runs%20Copilot%20on%20my%20own%20credits';

/** Banner asking for a Copilot token when needed; `open` shows it on demand to replace or remove the token. */
export function CopilotTokenPanel({ open, onClose }: { open: boolean; onClose: () => void }) {
  const qc = useQueryClient();
  const status = useQuery({
    queryKey: queryKeys.copilotToken,
    queryFn: ({ signal }) => api.getCopilotTokenStatus(signal),
    staleTime: 5 * 60_000,
  });
  const [token, setToken] = useState('');
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: queryKeys.copilotToken });
    void qc.invalidateQueries({ queryKey: queryKeys.copilotQuota });
    void qc.invalidateQueries({ queryKey: queryKeys.models });
  };
  const save = useMutation({
    mutationFn: () => api.saveCopilotToken(token.trim()),
    onSuccess: () => {
      setToken('');
      refresh();
      onClose();
    },
  });
  const remove = useMutation({ mutationFn: api.deleteCopilotToken, onSuccess: refresh });

  const s = status.data;
  if (!s || s.shared || (!s.needed && !open)) return null;
  const error = save.error && api.isApiError(save.error) ? (save.error.fieldErrors.token ?? save.error.message) : null;

  return (
    <Alert tone={s.needed ? 'warning' : 'info'} title={s.needed ? 'Add a Copilot token to run generations' : 'Copilot token'} className="mb-4">
      <p>
        Generations run on <strong>your own</strong> Copilot credits. Copilot does not accept classic tokens (ghp_), so
        add a separate Copilot token. Recommended (no organisation approval needed): run{' '}
        <code className="font-mono">gh auth login --web</code> once, then paste the output of{' '}
        <code className="font-mono">gh auth token</code> (starts with <code className="font-mono">gho_</code>). A
        fine-grained token with only the <strong>Copilot Requests</strong> permission also runs jobs, but may not list
        your models.{' '}
        <a href={NEW_TOKEN_URL} target="_blank" rel="noopener noreferrer" className={linkClass}>
          Create a fine-grained token
        </a>
      </p>
      <form
        className="flex flex-wrap items-start gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <input
          type="password"
          aria-label="Copilot token"
          placeholder="gho_… (gh auth token)"
          autoComplete="off"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          aria-invalid={Boolean(error)}
          className={`${inputClass} max-w-md font-mono`}
        />
        <Button type="submit" disabled={!token.trim()} loading={save.isPending}>
          {s.configured ? 'Replace token' : 'Save token'}
        </Button>
        {s.configured && (
          <Button variant="ghost" onClick={() => remove.mutate()} loading={remove.isPending}>
            Remove
          </Button>
        )}
        {open && !s.needed && (
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        )}
      </form>
      {error && <p className="text-red-700 dark:text-red-300">{error}</p>}
      {s.configured && <p>A Copilot token is saved; your generations use it.</p>}
    </Alert>
  );
}
