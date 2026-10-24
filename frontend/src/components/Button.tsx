import type { ButtonHTMLAttributes } from 'react';

import { buttonClass, type ButtonVariant } from '../lib/styles';
import { Spinner } from './Spinner';

type Props = ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant; loading?: boolean };

export function Button({ variant = 'primary', loading = false, className = '', children, disabled, ...rest }: Props) {
  return (
    <button
      type="button"
      className={buttonClass(variant, className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...rest}
    >
      {loading && <Spinner size="sm" />}
      {children}
    </button>
  );
}
