import type { ErrorCode } from '../api/types';

/** What the user can do about each error. Keep wording actionable; the server message says what happened. */
export const ERROR_GUIDANCE: Partial<Record<ErrorCode, string>> = {
  unauthenticated: 'Your session has ended. Sign in again to continue.',
  github_auth_expired: 'Your GitHub authorisation has expired. Sign in again to continue.',
  csrf_rejected: 'The request was blocked as a possible cross-site request. Reload the page and try again.',
  forbidden: 'You do not have permission for this action.',
  repository_access_denied:
    'You (or the portal’s GitHub App) no longer have the required access to this repository. Ask a repository admin for access.',
  app_not_installed: 'The portal’s GitHub App is not installed on this account. Install it, then try again.',
  model_unavailable:
    'This model is not available for your Copilot plan or your organisation’s model policy. Choose another model and generate again.',
  model_locked_by_agent: 'The agent pins its own model, so the model cannot be changed.',
  copilot_unauthorized:
    'Your GitHub account could not use GitHub Copilot through the portal. Check that you have an active Copilot seat, or contact your administrator.',
  agent_failed: 'The agent stopped with an error. Check the logs below; retrying may help.',
  agent_timeout:
    'The agent did not finish within the time limit. Retry, or narrow the analysis (for example to a subdirectory) if the agent supports it.',
  agent_unavailable: 'The agent runtime is temporarily unavailable. Try again in a few minutes.',
  output_missing: 'The agent finished without writing the expected documentation file. Check the logs and retry.',
  output_empty: 'The agent wrote an empty or too short document. Check the logs and retry.',
  output_invalid: 'The generated file did not pass validation. Check the logs and retry.',
  output_too_large: 'The generated document exceeds the size limit.',
  repository_too_large: 'The repository snapshot exceeds the portal’s size or file-count limit.',
  base_branch_changed:
    'The base branch changed after the documentation was generated, and the documentation file was modified in between. Start a new generation on the latest commit.',
  branch_conflict:
    'A branch with the portal’s name already exists with different history. The portal never force-pushes: delete or rename that branch on GitHub, then try again.',
  github_unavailable: 'GitHub is not responding right now. Try again shortly.',
  github_rate_limited: 'GitHub’s rate limit was reached. Wait a few minutes and try again.',
  rate_limited: 'Too many requests. Wait a moment and try again.',
  duplicate_job: 'A generation for this repository, branch and model is already in progress.',
  cancelled: 'The job was cancelled.',
  internal: 'Something went wrong on our side. Try again; if it keeps happening, contact the portal team.',
};

const AUTH_ERROR_MESSAGES: Record<string, string> = {
  access_denied: 'Sign-in was cancelled on GitHub.',
  unauthenticated: 'Sign-in could not be completed. Please try again.',
  csrf_rejected: 'The sign-in link expired or was opened in a different browser. Please try again.',
  github_unavailable: 'GitHub is not responding right now. Please try again shortly.',
  github_auth_expired: 'The GitHub authorisation expired before it could be used. Please try again.',
  forbidden: 'Your account is not allowed to use this portal.',
};

export function authErrorMessage(code: string): string {
  return AUTH_ERROR_MESSAGES[code] ?? `Sign-in failed (${code.slice(0, 64)}). Please try again.`;
}
