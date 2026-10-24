import { useAgent } from '../api/queries';

export function PlaceholderBanner() {
  const agent = useAgent();
  if (!agent.data?.placeholder) return null;
  return (
    <div
      className="border-b border-amber-300 bg-amber-50 px-4 py-2 text-center text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-100"
      role="status"
    >
      <strong>Placeholder agent installed.</strong> The portal is running a stand-in for the organisation’s
      bounded-context agent. Generated documents are for testing only.
    </div>
  );
}
