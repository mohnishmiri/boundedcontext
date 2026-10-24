import { Button } from './Button';

export function Pagination({
  page,
  hasMore,
  onPageChange,
  label,
  disabled = false,
}: {
  page: number;
  hasMore: boolean;
  onPageChange: (page: number) => void;
  label: string;
  disabled?: boolean;
}) {
  if (page <= 1 && !hasMore) return null;
  return (
    <nav aria-label={label} className="flex items-center justify-between gap-2 pt-3">
      <Button variant="secondary" disabled={disabled || page <= 1} onClick={() => onPageChange(page - 1)}>
        Previous
      </Button>
      <span className="text-sm text-gray-600 dark:text-gray-300">Page {page}</span>
      <Button variant="secondary" disabled={disabled || !hasMore} onClick={() => onPageChange(page + 1)}>
        Next
      </Button>
    </nav>
  );
}
