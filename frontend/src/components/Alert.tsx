import type { ReactNode } from 'react';

export type AlertTone = 'info' | 'warning' | 'error' | 'success';

const tones: Record<AlertTone, string> = {
  info: 'border-blue-200 bg-blue-50 text-blue-900 dark:border-blue-900 dark:bg-blue-950/50 dark:text-blue-100',
  warning: 'border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-800 dark:bg-amber-950/50 dark:text-amber-100',
  error: 'border-red-300 bg-red-50 text-red-900 dark:border-red-900 dark:bg-red-950/50 dark:text-red-100',
  success: 'border-green-300 bg-green-50 text-green-900 dark:border-green-900 dark:bg-green-950/50 dark:text-green-100',
};

export function Alert({
  tone = 'info',
  title,
  children,
  role,
  className = '',
}: {
  tone?: AlertTone;
  title?: ReactNode;
  children?: ReactNode;
  role?: 'alert' | 'status';
  className?: string;
}) {
  return (
    <div className={`rounded-md border px-4 py-3 text-sm ${tones[tone]} ${className}`} role={role}>
      {title && <p className="font-semibold">{title}</p>}
      {children && <div className={title ? 'mt-1 space-y-2' : 'space-y-2'}>{children}</div>}
    </div>
  );
}
