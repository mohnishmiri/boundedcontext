import { inputClass } from '../lib/styles';
import { Button } from './Button';

const numberFormat = new Intl.NumberFormat();

/** Page numbers to show: first, last and a window around the current page; `null` marks a gap. */
function pageWindow(page: number, pageCount: number): (number | null)[] {
  const wanted = new Set([1, pageCount, page - 1, page, page + 1].filter((p) => p >= 1 && p <= pageCount));
  const sorted = [...wanted].sort((a, b) => a - b);
  const out: (number | null)[] = [];
  sorted.forEach((p, i) => {
    if (i > 0 && p - sorted[i - 1] > 1) out.push(p - sorted[i - 1] === 2 ? p - 1 : null);
    out.push(p);
  });
  return out;
}

export function Pagination({
  page,
  hasMore,
  onPageChange,
  label,
  disabled = false,
  total,
  perPage,
  perPageOptions,
  onPerPageChange,
}: {
  page: number;
  hasMore: boolean;
  onPageChange: (page: number) => void;
  label: string;
  disabled?: boolean;
  /** Total number of matching rows; enables the range summary and numbered pages. */
  total?: number;
  perPage?: number;
  perPageOptions?: number[];
  onPerPageChange?: (perPage: number) => void;
}) {
  const known = total !== undefined && perPage !== undefined && perPage > 0;
  const pageCount = known ? Math.max(1, Math.ceil(total / perPage)) : undefined;
  if (!known && page <= 1 && !hasMore) return null;
  if (known && total === 0) return null;

  const from = known ? Math.min(total, (page - 1) * perPage + 1) : 0;
  const to = known ? Math.min(total, page * perPage) : 0;
  const canNext = pageCount ? page < pageCount : hasMore;
  const sizeId = `${label.replace(/\W+/g, '-').toLowerCase()}-size`;

  return (
    <nav aria-label={label} className="flex flex-wrap items-center justify-between gap-3 pt-3">
      <div className="flex flex-wrap items-center gap-3 text-sm text-gray-600 dark:text-gray-300">
        <span aria-live="polite">
          {known
            ? `Showing ${numberFormat.format(from)}–${numberFormat.format(to)} of ${numberFormat.format(total)}`
            : `Page ${page}`}
        </span>
        {perPage !== undefined && perPageOptions && onPerPageChange && (
          <span className="flex items-center gap-2">
            <label htmlFor={sizeId}>Rows per page</label>
            <select
              id={sizeId}
              value={perPage}
              disabled={disabled}
              onChange={(e) => onPerPageChange(Number(e.target.value))}
              className={`${inputClass} w-20 py-1`}
            >
              {perPageOptions.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </span>
        )}
      </div>
      {(page > 1 || canNext) && (
        <div className="flex flex-wrap items-center gap-1">
          {pageCount && (
            <Button variant="ghost" aria-label="First page" disabled={disabled || page <= 1} onClick={() => onPageChange(1)}>
              «
            </Button>
          )}
          <Button variant="secondary" disabled={disabled || page <= 1} onClick={() => onPageChange(page - 1)}>
            Previous
          </Button>
          {pageCount &&
            pageWindow(page, pageCount).map((p, i) =>
              p === null ? (
                <span key={`gap-${i}`} className="px-1 text-sm text-gray-500" aria-hidden="true">
                  …
                </span>
              ) : (
                <Button
                  key={p}
                  variant={p === page ? 'primary' : 'ghost'}
                  aria-label={`Page ${p}`}
                  aria-current={p === page ? 'page' : undefined}
                  disabled={disabled && p !== page}
                  onClick={() => p !== page && onPageChange(p)}
                  className="min-w-[2.25rem] tabular-nums"
                >
                  {p}
                </Button>
              ),
            )}
          <Button variant="secondary" disabled={disabled || !canNext} onClick={() => onPageChange(page + 1)}>
            Next
          </Button>
          {pageCount && (
            <Button
              variant="ghost"
              aria-label="Last page"
              disabled={disabled || page >= pageCount}
              onClick={() => onPageChange(pageCount)}
            >
              »
            </Button>
          )}
        </div>
      )}
    </nav>
  );
}
