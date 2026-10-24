import type { Step, StepState } from '../api/types';
import { formatDuration, formatTime } from '../lib/format';

const stateText: Record<StepState, string> = {
  pending: 'Pending',
  active: 'In progress',
  done: 'Done',
  failed: 'Failed',
  skipped: 'Skipped',
};

function StepIcon({ state }: { state: StepState }) {
  const common = 'flex h-6 w-6 shrink-0 items-center justify-center rounded-full';
  switch (state) {
    case 'done':
      return (
        <span className={`${common} bg-green-600 text-white`} aria-hidden="true">
          <svg viewBox="0 0 20 20" className="h-4 w-4" fill="currentColor">
            <path d="M7.7 13.3 4.4 10l-1.1 1.1 4.4 4.4 9-9-1.1-1.1z" />
          </svg>
        </span>
      );
    case 'failed':
      return (
        <span className={`${common} bg-red-600 text-white`} aria-hidden="true">
          <svg viewBox="0 0 20 20" className="h-4 w-4" fill="currentColor">
            <path d="m5.6 4.5 4.4 4.4 4.4-4.4 1.1 1.1-4.4 4.4 4.4 4.4-1.1 1.1-4.4-4.4-4.4 4.4-1.1-1.1 4.4-4.4-4.4-4.4z" />
          </svg>
        </span>
      );
    case 'active':
      return (
        <span className={`${common} border-2 border-blue-600 dark:border-blue-400`} aria-hidden="true">
          <span className="h-2.5 w-2.5 animate-pulse rounded-full bg-blue-600 dark:bg-blue-400" />
        </span>
      );
    case 'skipped':
      return (
        <span className={`${common} border-2 border-dashed border-gray-300 dark:border-gray-600`} aria-hidden="true">
          <span className="h-0.5 w-2.5 bg-gray-400" />
        </span>
      );
    default:
      return <span className={`${common} border-2 border-gray-300 dark:border-gray-600`} aria-hidden="true" />;
  }
}

function timing(step: Step): string {
  if (step.started_at && step.finished_at) {
    return formatDuration(new Date(step.finished_at).getTime() - new Date(step.started_at).getTime());
  }
  if (step.started_at) return `started ${formatTime(step.started_at)}`;
  return '';
}

export function StepList({ steps, label }: { steps: Step[]; label: string }) {
  if (steps.length === 0) return null;
  return (
    <ol aria-label={label} className="space-y-3">
      {steps.map((step) => {
        const t = timing(step);
        return (
          <li
            key={step.key}
            className="flex items-start gap-3"
            aria-current={step.state === 'active' ? 'step' : undefined}
            data-state={step.state}
          >
            <StepIcon state={step.state} />
            <div className="min-w-0">
              <p
                className={
                  step.state === 'pending' || step.state === 'skipped'
                    ? 'text-sm text-gray-500 dark:text-gray-400'
                    : step.state === 'failed'
                      ? 'text-sm font-medium text-red-700 dark:text-red-300'
                      : 'text-sm font-medium text-gray-900 dark:text-gray-100'
                }
              >
                {step.label}
              </p>
              <p className="text-xs text-gray-500 dark:text-gray-400">
                {stateText[step.state]}
                {t && ` · ${t}`}
              </p>
            </div>
          </li>
        );
      })}
    </ol>
  );
}
