import { useState } from 'react';

import { downloadXlsx, exportFileName, type Sheet } from '../lib/xlsx';
import { Button } from './Button';

function ExcelIcon() {
  return (
    <svg viewBox="0 0 20 20" className="h-4 w-4" fill="currentColor" aria-hidden="true">
      <path d="M4.75 2A1.75 1.75 0 0 0 3 3.75v12.5c0 .966.784 1.75 1.75 1.75h10.5A1.75 1.75 0 0 0 17 16.25V7.5a.75.75 0 0 0-.22-.53l-4.75-4.75A.75.75 0 0 0 11.5 2H4.75Zm2.28 6.47a.75.75 0 0 0-1.06 1.06L7.94 11.5l-1.97 1.97a.75.75 0 1 0 1.06 1.06L9 12.56l1.97 1.97a.75.75 0 1 0 1.06-1.06l-1.97-1.97 1.97-1.97a.75.75 0 0 0-1.06-1.06L9 10.44 7.03 8.47Z" />
    </svg>
  );
}

/** Builds the workbook on click (possibly fetching every page first) and downloads it as `.xlsx`. */
export function ExportButton({
  fileBase,
  build,
  label = 'Export to Excel',
  disabled,
  className,
}: {
  fileBase: string;
  build: () => Sheet[] | Promise<Sheet[]>;
  label?: string;
  disabled?: boolean;
  className?: string;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      downloadXlsx(exportFileName(fileBase), await build());
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Export failed');
    } finally {
      setBusy(false);
    }
  };

  return (
    <span className="inline-flex flex-col items-end gap-1">
      <Button variant="secondary" onClick={run} loading={busy} disabled={disabled} className={className}>
        {!busy && <ExcelIcon />}
        {label}
      </Button>
      {error && (
        <span role="alert" className="text-xs text-red-700 dark:text-red-300">
          {error}
        </span>
      )}
    </span>
  );
}
