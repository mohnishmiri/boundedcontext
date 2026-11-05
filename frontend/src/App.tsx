import { QueryClientProvider, useQueryClient } from '@tanstack/react-query';
import { lazy, Suspense, useEffect, useState, type ReactNode } from 'react';
import { BrowserRouter, Link, Route, Routes } from 'react-router-dom';

import { setUnauthorizedHandler } from './api/client';
import { queryKeys, useMe } from './api/queries';
import { Button } from './components/Button';
import { ErrorPanel } from './components/ErrorPanel';
import { Layout } from './components/Layout';
import { PageSpinner } from './components/Spinner';
import { SignInPage } from './features/auth/SignInPage';
import { DashboardPage } from './features/dashboard/DashboardPage';
import { createQueryClient } from './lib/queryClient';
import { ROUTER_FUTURE } from './lib/router';
import { linkClass } from './lib/styles';

// Loaded on demand: the job page pulls in the Markdown renderer, the new-job page the repository pickers.
const JobPage = lazy(() => import('./features/job/JobPage').then((m) => ({ default: m.JobPage })));
const NewJobPage = lazy(() => import('./features/new-job/NewJobPage').then((m) => ({ default: m.NewJobPage })));
const BatchPage = lazy(() => import('./features/batch/BatchPage').then((m) => ({ default: m.BatchPage })));

function AuthGate({ children }: { children: (user: NonNullable<ReturnType<typeof useMe>['data']>) => ReactNode }) {
  const qc = useQueryClient();
  const me = useMe();

  useEffect(() => {
    setUnauthorizedHandler((err) => {
      qc.setQueryData(queryKeys.authNotice, err.code);
      qc.setQueryData(queryKeys.me, null);
    });
    return () => setUnauthorizedHandler(null);
  }, [qc]);

  if (me.isPending) return <PageSpinner />;
  if (me.isError) {
    return (
      <div className="mx-auto max-w-lg p-8">
        <ErrorPanel error={me.error} title="The portal is not reachable">
          <Button variant="secondary" onClick={() => me.refetch()}>
            Try again
          </Button>
        </ErrorPanel>
      </div>
    );
  }
  if (!me.data) return <SignInPage />;
  return <>{children(me.data)}</>;
}

function NotFoundPage() {
  return (
    <div className="space-y-3">
      <h1 className="text-xl font-semibold">Page not found</h1>
      <Link to="/" className={linkClass}>
        Back to your jobs
      </Link>
    </div>
  );
}

/** Everything below the router; rendered by tests inside a MemoryRouter. */
export function AppRoutes() {
  return (
    <AuthGate>
      {(user) => (
        <Layout user={user}>
          <Suspense fallback={<PageSpinner />}>
            <Routes>
              <Route path="/" element={<DashboardPage />} />
              <Route path="/jobs/new" element={<NewJobPage />} />
              <Route path="/batches/:batchId" element={<BatchPage />} />
              <Route path="/jobs/:jobId" element={<JobPage />} />
              <Route path="*" element={<NotFoundPage />} />
            </Routes>
          </Suspense>
        </Layout>
      )}
    </AuthGate>
  );
}

export default function App() {
  const [queryClient] = useState(() => createQueryClient());
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter future={ROUTER_FUTURE}>
        <AppRoutes />
      </BrowserRouter>
    </QueryClientProvider>
  );
}
