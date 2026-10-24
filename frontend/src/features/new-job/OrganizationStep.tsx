import { useOrganizations } from '../../api/queries';
import { Alert } from '../../components/Alert';
import { ErrorPanel } from '../../components/ErrorPanel';
import { Spinner } from '../../components/Spinner';
import { safeExternalUrl } from '../../lib/format';
import { linkClass } from '../../lib/styles';

export function OrganizationStep({
  owner,
  onChange,
  error,
}: {
  owner: string | null;
  onChange: (owner: string) => void;
  error?: string;
}) {
  const orgs = useOrganizations();
  const installUrl = safeExternalUrl(orgs.data?.install_url);
  const installLink = installUrl && (
    <a href={installUrl} target="_blank" rel="noopener noreferrer" className={linkClass}>
      Install the GitHub App on another account
    </a>
  );

  return (
    <fieldset className="space-y-3">
      <legend className="text-base font-semibold">1. Account</legend>
      {orgs.isPending && <Spinner label="Loading accounts…" />}
      {orgs.isError && <ErrorPanel error={orgs.error} title="Could not load your GitHub accounts" />}
      {orgs.data && orgs.data.items.length === 0 && (
        <Alert tone="warning" title="No accounts available">
          <p>The portal’s GitHub App is not installed on any account you can access.</p>
          {installLink}
        </Alert>
      )}
      {orgs.data && orgs.data.items.length > 0 && (
        <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
          {orgs.data.items.map((org) => {
            const avatar = safeExternalUrl(org.avatar_url);
            const checked = owner === org.login;
            return (
              <label
                key={org.installation_id}
                className={`flex cursor-pointer items-center gap-3 rounded-md border px-3 py-2 text-sm focus-within:ring-2 focus-within:ring-indigo-500 ${
                  checked
                    ? 'border-indigo-500 bg-indigo-50 dark:bg-indigo-950/40'
                    : 'border-gray-200 hover:bg-gray-50 dark:border-gray-700 dark:hover:bg-gray-800'
                }`}
              >
                <input
                  type="radio"
                  name="owner"
                  value={org.login}
                  checked={checked}
                  onChange={() => onChange(org.login)}
                  className="sr-only"
                />
                {avatar ? (
                  <img src={avatar} alt="" className="h-6 w-6 rounded" referrerPolicy="no-referrer" />
                ) : (
                  <span className="h-6 w-6 rounded bg-gray-200 dark:bg-gray-700" aria-hidden="true" />
                )}
                <span className="min-w-0">
                  <span className="block truncate font-medium">{org.login}</span>
                  <span className="block text-xs text-gray-500">
                    {org.type === 'User' ? 'Personal account' : 'Organization'}
                  </span>
                </span>
              </label>
            );
          })}
        </div>
      )}
      {error && (
        <p className="text-sm text-red-600 dark:text-red-400" role="alert">
          {error}
        </p>
      )}
      {orgs.data && orgs.data.items.length > 0 && installLink && (
        <p className="text-xs text-gray-600 dark:text-gray-400">Organisation missing? {installLink}.</p>
      )}
    </fieldset>
  );
}
