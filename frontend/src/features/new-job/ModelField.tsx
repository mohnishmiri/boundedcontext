import type { ModelList } from '../../api/types';
import { inputClass } from '../../lib/styles';

export function ModelField({
  models,
  value,
  onChange,
  error,
  lockedModel = null,
  lockReason = null,
}: {
  models: ModelList;
  value: string;
  onChange: (model: string) => void;
  error?: string;
  /** Model pinned by the selected generator's agent; disables the selector. */
  lockedModel?: string | null;
  lockReason?: string | null;
}) {
  const locked = lockedModel;
  const options = [...models.items];
  if (locked && !options.some((m) => m.id === locked)) {
    options.unshift({ id: locked, name: locked, vendor: 'Pinned by generator', description: null, multiplier: null, is_default: false });
  }
  const selected = options.find((m) => m.id === value);
  const describedBy = ['model-help', locked ? 'model-lock' : null, error ? 'model-error' : null].filter(Boolean).join(' ');

  return (
    <div className="space-y-2">
      <label htmlFor="model" className="block text-base font-semibold">
        4. Model
      </label>
      <select
        id="model"
        className={`${inputClass} sm:max-w-md`}
        value={value}
        disabled={Boolean(locked)}
        onChange={(e) => onChange(e.target.value)}
        aria-describedby={describedBy}
        aria-invalid={error ? true : undefined}
      >
        {options.map((m) => (
          <option key={m.id} value={m.id}>
            {m.name} — {m.vendor}
            {m.multiplier !== null ? ` (×${m.multiplier})` : ''}
            {m.id === models.default_model ? ' · default' : ''}
          </option>
        ))}
      </select>
      {locked && (
        <p id="model-lock" className="text-sm text-amber-800 dark:text-amber-200">
          {lockReason ?? 'The generator pins its model, so it cannot be changed here.'}
        </p>
      )}
      {selected?.description && <p className="text-sm text-gray-600 dark:text-gray-300">{selected.description}</p>}
      <p id="model-help" className="text-xs text-gray-500 dark:text-gray-400">
        {models.entitlement_note}
        {options.some((m) => m.multiplier !== null) && ' “×n” is the premium-request multiplier.'}
      </p>
      {error && (
        <p id="model-error" className="text-sm text-red-600 dark:text-red-400" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
