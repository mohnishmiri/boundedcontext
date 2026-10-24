import { useQueryClient } from '@tanstack/react-query';
import { useLocation, useSearchParams } from 'react-router-dom';

import { signInUrl } from '../../api/client';
import { queryKeys } from '../../api/queries';
import { Alert } from '../../components/Alert';
import { authErrorMessage, ERROR_GUIDANCE } from '../../lib/errors';
import { buttonClass } from '../../lib/styles';

export function SignInPage() {
  const qc = useQueryClient();
  const location = useLocation();
  const [params] = useSearchParams();
  const authError = params.get('auth_error');
  const notice = qc.getQueryData<string>(queryKeys.authNotice);

  const returnParams = new URLSearchParams(location.search);
  returnParams.delete('auth_error');
  const qs = returnParams.toString();
  const returnTo = `${location.pathname}${qs ? `?${qs}` : ''}`;

  return (
    <div className="flex min-h-screen items-center justify-center bg-gray-50 px-4 dark:bg-gray-950">
      <main className="w-full max-w-md space-y-6 rounded-lg border border-gray-200 bg-white p-8 shadow-sm dark:border-gray-800 dark:bg-gray-900">
        <div className="flex items-center gap-3">
          <img src="/favicon.svg" alt="" className="h-10 w-10" />
          <h1 className="text-xl font-semibold text-gray-900 dark:text-white">Bounded Context Portal</h1>
        </div>
        <p className="text-sm text-gray-600 dark:text-gray-300">
          Generate bounded-context documentation for a GitHub repository with the organisation’s agent, review the
          result, and open a pull request.
        </p>
        {authError && (
          <Alert tone="error" role="alert">
            {authErrorMessage(authError)}
          </Alert>
        )}
        {!authError && (notice === 'github_auth_expired' || notice === 'unauthenticated') && (
          <Alert tone="warning" role="status">
            {ERROR_GUIDANCE[notice]}
          </Alert>
        )}
        <a href={signInUrl(returnTo)} className={buttonClass('primary', 'w-full py-2.5')}>
          <svg viewBox="0 0 16 16" className="h-5 w-5" fill="currentColor" aria-hidden="true">
            <path d="M8 0C3.58 0 0 3.58 0 8a8 8 0 0 0 5.47 7.59c.4.07.55-.17.55-.38v-1.33c-2.23.48-2.7-1.07-2.7-1.07-.36-.92-.89-1.17-.89-1.17-.73-.5.06-.49.06-.49.8.06 1.23.83 1.23.83.72 1.22 1.87.87 2.33.66.07-.52.28-.87.5-1.07-1.78-.2-3.65-.89-3.65-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82a7.6 7.6 0 0 1 4 0c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48v2.2c0 .21.15.46.55.38A8 8 0 0 0 16 8c0-4.42-3.58-8-8-8z" />
          </svg>
          Sign in with GitHub
        </a>
        <p className="text-xs text-gray-500 dark:text-gray-400">
          You will only see repositories that you can access and where the portal’s GitHub App is installed.
        </p>
      </main>
    </div>
  );
}
