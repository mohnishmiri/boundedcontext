/** Warns that the selected generator is a stand-in whose output is for testing only. */
export function PlaceholderBanner({ name }: { name?: string }) {
  return (
    <div
      className="rounded-md border border-amber-300 bg-amber-50 px-4 py-2 text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-100"
      role="status"
    >
      <strong>Placeholder generator.</strong> {name ? <>“{name}” is</> : 'This generator is'} a stand-in for the
      organisation’s bounded-context agent. Generated documents are for testing only.
    </div>
  );
}
