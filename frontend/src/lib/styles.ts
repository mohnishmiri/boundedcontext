export type ButtonVariant = 'primary' | 'secondary' | 'danger' | 'ghost';

const base =
  'inline-flex items-center justify-center gap-2 rounded-md px-3.5 py-2 text-sm font-medium transition-colors ' +
  'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-500 ' +
  'disabled:cursor-not-allowed disabled:opacity-50';

const variants: Record<ButtonVariant, string> = {
  primary: 'bg-indigo-600 text-white shadow-sm hover:bg-indigo-500 disabled:hover:bg-indigo-600',
  secondary:
    'bg-white text-gray-900 shadow-sm ring-1 ring-inset ring-gray-300 hover:bg-gray-50 ' +
    'dark:bg-gray-800 dark:text-gray-100 dark:ring-gray-600 dark:hover:bg-gray-700',
  danger: 'bg-red-600 text-white shadow-sm hover:bg-red-500 disabled:hover:bg-red-600',
  ghost: 'text-gray-700 hover:bg-gray-100 dark:text-gray-200 dark:hover:bg-gray-800',
};

/** Classes for buttons and button-styled links. */
export function buttonClass(variant: ButtonVariant = 'primary', extra = ''): string {
  return `${base} ${variants[variant]} ${extra}`.trim();
}

export const cardClass =
  'rounded-lg border border-gray-200 bg-white p-5 shadow-sm dark:border-gray-700 dark:bg-gray-900';

export const inputClass =
  'block w-full rounded-md border-0 bg-white px-3 py-2 text-sm text-gray-900 shadow-sm ring-1 ring-inset ring-gray-300 ' +
  'placeholder:text-gray-400 focus:ring-2 focus:ring-inset focus:ring-indigo-500 disabled:cursor-not-allowed ' +
  'disabled:bg-gray-100 disabled:text-gray-500 dark:bg-gray-800 dark:text-gray-100 dark:ring-gray-600 ' +
  'dark:disabled:bg-gray-800/60 aria-[invalid=true]:ring-red-500';

export const linkClass =
  'font-medium text-indigo-600 underline-offset-2 hover:underline focus-visible:outline focus-visible:outline-2 ' +
  'focus-visible:outline-offset-2 focus-visible:outline-indigo-500 dark:text-indigo-400';
