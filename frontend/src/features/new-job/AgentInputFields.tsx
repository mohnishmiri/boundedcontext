import type { AgentInput } from '../../api/types';
import { inputClass } from '../../lib/styles';

export function AgentInputFields({
  inputs,
  values,
  errors,
  onChange,
}: {
  inputs: AgentInput[];
  values: Record<string, string>;
  errors: Record<string, string | undefined>;
  onChange: (name: string, value: string) => void;
}) {
  if (inputs.length === 0) return null;
  return (
    <fieldset className="space-y-4">
      <legend className="text-base font-semibold">5. Agent options</legend>
      {inputs.map((input) => {
        const id = `input-${input.name}`;
        const error = errors[input.name];
        const describedBy = [input.help ? `${id}-help` : null, error ? `${id}-error` : null].filter(Boolean).join(' ');
        const common = {
          id,
          name: input.name,
          value: values[input.name] ?? '',
          required: input.required,
          'aria-invalid': error ? true : undefined,
          'aria-describedby': describedBy || undefined,
        };
        return (
          <div key={input.name} className="space-y-1">
            <label htmlFor={id} className="block text-sm font-medium">
              {input.label}
              {input.required && (
                <span className="text-red-600" aria-hidden="true">
                  {' '}
                  *
                </span>
              )}
            </label>
            {input.kind === 'select' ? (
              <select
                {...common}
                className={`${inputClass} sm:max-w-md`}
                onChange={(e) => onChange(input.name, e.target.value)}
              >
                {!input.required && <option value="">—</option>}
                {(input.choices ?? []).map((c) => (
                  <option key={c} value={c}>
                    {c}
                  </option>
                ))}
              </select>
            ) : (
              <input
                {...common}
                type="text"
                maxLength={input.max_length}
                spellCheck={input.kind === 'path' ? false : undefined}
                autoComplete="off"
                placeholder={input.kind === 'path' ? 'e.g. services/orders' : undefined}
                className={`${inputClass} sm:max-w-md ${input.kind === 'path' ? 'font-mono' : ''}`}
                onChange={(e) => onChange(input.name, e.target.value)}
              />
            )}
            {input.help && (
              <p id={`${id}-help`} className="text-xs text-gray-500 dark:text-gray-400">
                {input.help}
              </p>
            )}
            {error && (
              <p id={`${id}-error`} className="text-sm text-red-600 dark:text-red-400" role="alert">
                {error}
              </p>
            )}
          </div>
        );
      })}
    </fieldset>
  );
}
