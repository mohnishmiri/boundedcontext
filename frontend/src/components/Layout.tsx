import { useMutation, useQueryClient } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { Link, NavLink, useNavigate } from 'react-router-dom';

import * as api from '../api/client';
import { queryKeys } from '../api/queries';
import type { User } from '../api/types';
import { safeExternalUrl } from '../lib/format';
import { Button } from './Button';

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
        {children}
      </main>
    </div>
  );
}
