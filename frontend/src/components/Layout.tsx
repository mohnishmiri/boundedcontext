import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { Link, NavLink, useNavigate } from 'react-router-dom';

import * as api from '../api/client';
import { queryKeys, useCopilotQuota } from '../api/queries';
import type { User } from '../api/types';
import { formatDateTime, safeExternalUrl } from '../lib/format';
import { Button } from './Button';
import { CopilotTokenPanel } from './CopilotTokenPanel';

const credits = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 });

function QuotaMeter() {
  const { data } = useCopilotQuota();
  if (!data?.available) return null;
  const label = data.unit === 'ai_credits' ? 'AI credits' : 'Premium requests';
  const reset = data.reset_date ? `Resets ${formatDateTime(data.reset_date)}` : undefined;
  if (data.unlimited) {
    return (
      <span className="text-xs text-gray-600 dark:text-gray-300" title={reset}>
        {label}: unlimited
      </span>
    );
  }
  if (data.used === null || data.entitlement === null) return null;
  const pctUsed = data.entitlement > 0 ? Math.min(100, (data.used / data.entitlement) * 100) : 0;
  const bar = pctUsed >= 90 ? 'bg-red-500' : pctUsed >= 75 ? 'bg-amber-500' : 'bg-emerald-500';
  return (
    <div className="flex flex-col gap-1 text-xs text-gray-600 dark:text-gray-300" title={reset}>
      <span>
        {label}: <strong className="text-gray-900 dark:text-white">{credits.format(data.used)}</strong> of{' '}
        {credits.format(data.entitlement)} used
        {data.percent_remaining !== null && ` (${data.percent_remaining.toFixed(1)}% left)`}
      </span>
      <span
        role="progressbar"
        aria-label={`${label} used`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pctUsed)}
        className="h-1.5 w-48 overflow-hidden rounded bg-gray-200 dark:bg-gray-700"
      >
        <span className={`block h-full ${bar}`} style={{ width: `${pctUsed}%` }} />
      </span>
    </div>
  );
}

const navClass = ({ isActive }: { isActive: boolean }) =>
  `rounded-md px-3 py-2 text-sm font-medium focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500 ${
    isActive
      ? 'bg-gray-100 text-gray-900 dark:bg-gray-800 dark:text-white'
      : 'text-gray-600 hover:bg-gray-50 hover:text-gray-900 dark:text-gray-300 dark:hover:bg-gray-800 dark:hover:text-white'
  }`;

export function Layout({ user, children }: { user: User; children: ReactNode }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const logout = useMutation({
    mutationFn: api.logout,
    onSettled: () => {
      qc.removeQueries({ predicate: (q) => q.queryKey[0] !== 'auth' });
      qc.setQueryData(queryKeys.me, null);
      navigate('/', { replace: true });
    },
  });
  const avatar = safeExternalUrl(user.avatar_url);
  const [tokenPanelOpen, setTokenPanelOpen] = useState(false);

  return (
    <div className="min-h-screen bg-gray-50 text-gray-900 dark:bg-gray-950 dark:text-gray-100">
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:absolute focus:left-2 focus:top-2 focus:z-50 focus:rounded focus:bg-white focus:px-3 focus:py-2 focus:text-sm focus:shadow"
      >
        Skip to content
      </a>
      <header className="border-b border-gray-200 bg-white dark:border-gray-800 dark:bg-gray-900">
        <div className="mx-auto flex max-w-6xl flex-wrap items-center justify-between gap-3 px-4 py-3">
          <div className="flex items-center gap-6">
            <Link to="/" className="flex items-center gap-2 font-semibold">
              <img src="/favicon.svg" alt="" className="h-7 w-7" />
              <span>Bounded Context Portal</span>
            </Link>
            <nav aria-label="Main" className="flex gap-1">
              <NavLink to="/" end className={navClass}>
                Jobs
              </NavLink>
              <NavLink to="/jobs/new" className={navClass}>
                New generation
              </NavLink>
            </nav>
          </div>
          <div className="flex items-center gap-3">
            <QuotaMeter />
            <Button variant="ghost" onClick={() => setTokenPanelOpen((o) => !o)}>
              Copilot token
            </Button>
            {avatar && <img src={avatar} alt="" className="h-7 w-7 rounded-full" referrerPolicy="no-referrer" />}
            <span className="text-sm" title={user.name ?? undefined}>
              {user.login}
            </span>
            <Button variant="ghost" onClick={() => logout.mutate()} loading={logout.isPending}>
              Sign out
            </Button>
          </div>
        </div>
      </header>
      <main id="main" className="mx-auto max-w-6xl px-4 py-6" tabIndex={-1}>
        <CopilotTokenPanel open={tokenPanelOpen} onClose={() => setTokenPanelOpen(false)} />
        {children}
      </main>
    </div>
  );
}
