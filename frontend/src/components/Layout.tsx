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
      <span className="text-xs text-white/80" title={reset}>
        {label}: unlimited
      </span>
    );
  }
  if (data.used === null || data.entitlement === null) return null;
  const pctUsed = data.entitlement > 0 ? Math.min(100, (data.used / data.entitlement) * 100) : 0;
  const bar = pctUsed >= 90 ? 'bg-red-500' : pctUsed >= 75 ? 'bg-att-orange' : 'bg-att';
  return (
    <div className="flex flex-col gap-1 text-xs text-white/80" title={reset}>
      <span>
        {label}: <strong className="text-white">{credits.format(data.used)}</strong> of{' '}
        {credits.format(data.entitlement)} used
        {data.percent_remaining !== null && ` (${data.percent_remaining.toFixed(1)}% left)`}
      </span>
      <span
        role="progressbar"
        aria-label={`${label} used`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pctUsed)}
        className="h-1.5 w-48 overflow-hidden rounded-full bg-white/20"
      >
        <span className={`block h-full ${bar}`} style={{ width: `${pctUsed}%` }} />
      </span>
    </div>
  );
}

const navClass = ({ isActive }: { isActive: boolean }) =>
  `rounded-md px-3 py-2 text-sm font-medium transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-white ${
    isActive ? 'bg-white/15 text-white shadow-inner' : 'text-white/80 hover:bg-white/10 hover:text-white'
  }`;

function Footer() {
  return (
    <footer className="mt-10 bg-[#1f4e6e] text-white">
      <div className="mx-auto flex max-w-6xl flex-wrap items-center justify-between gap-3 px-4 py-3">
        <div className="flex items-center gap-2">
          {/* The logo file is the globe plus wordmark; show only the globe, in white. */}
          <img src="/att-logo.svg" alt="AT&T" className="h-6 w-6 object-cover object-left brightness-0 invert" />
          <span className="text-sm font-semibold">AT&amp;T Bounded Context Portal</span>
        </div>
        <p className="text-xs text-white/80">
          © {new Date().getFullYear()} AT&amp;T Intellectual Property. All rights reserved.
        </p>
        <span className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-[11px] text-white/90 ring-1 ring-inset ring-white/20">
          v{__APP_VERSION__}
        </span>
      </div>
    </footer>
  );
}

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
    <div className="flex min-h-screen flex-col bg-gray-50 text-gray-900 dark:bg-gray-950 dark:text-gray-100">
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:absolute focus:left-2 focus:top-2 focus:z-50 focus:rounded focus:bg-white focus:px-3 focus:py-2 focus:text-sm focus:shadow"
      >
        Skip to content
      </a>
      <header className="border-b-[3px] border-att bg-gradient-to-r from-att-900 via-att-700 to-att-600 text-white shadow-md">
        <div className="mx-auto flex max-w-6xl flex-wrap items-center justify-between gap-3 px-4 py-3">
          <div className="flex items-center gap-6">
            <Link
              to="/"
              className="flex items-center gap-2.5 rounded font-semibold tracking-tight focus-visible:outline focus-visible:outline-2 focus-visible:outline-white"
            >
              <img src="/favicon.svg" alt="" className="h-8 w-8 rounded-[7px] shadow-sm ring-1 ring-white/40" />
              <span className="flex flex-col leading-tight">
                <span className="text-base">Bounded Context Portal</span>
                <span className="text-[11px] font-normal uppercase tracking-wider text-white/70">AIFC Documentation</span>
              </span>
            </Link>
            <span className="hidden h-8 w-px bg-white/20 md:block" aria-hidden="true" />
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
            <Button variant="inverse" onClick={() => setTokenPanelOpen((o) => !o)}>
              Copilot token
            </Button>
            {avatar && (
              <img src={avatar} alt="" className="h-7 w-7 rounded-full ring-2 ring-white/60" referrerPolicy="no-referrer" />
            )}
            <span className="text-sm" title={user.name ?? undefined}>
              {user.login}
            </span>
            {user.is_admin && (
              <span
                className="rounded-full bg-att px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wide text-black"
                title="Can view every user's jobs"
              >
                Admin
              </span>
            )}
            <Button variant="inverse" onClick={() => logout.mutate()} loading={logout.isPending}>
              Sign out
            </Button>
          </div>
        </div>
      </header>
      <main id="main" className="mx-auto w-full max-w-6xl flex-1 px-4 py-6" tabIndex={-1}>
        <CopilotTokenPanel open={tokenPanelOpen} onClose={() => setTokenPanelOpen(false)} />
        {children}
      </main>
      <Footer />
    </div>
  );
}
