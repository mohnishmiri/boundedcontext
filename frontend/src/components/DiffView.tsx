import { useMemo } from 'react';

import { parseUnifiedDiff, type DiffLineKind } from '../lib/diff';

const rowStyles: Record<DiffLineKind, string> = {
  add: 'bg-green-50 text-green-900 dark:bg-green-950/60 dark:text-green-100',
  del: 'bg-red-50 text-red-900 dark:bg-red-950/60 dark:text-red-100',
  context: 'text-gray-800 dark:text-gray-200',
  hunk: 'bg-indigo-50 text-indigo-800 dark:bg-indigo-950/60 dark:text-indigo-200',
  meta: 'text-gray-500 dark:text-gray-400',
};

const markers: Record<DiffLineKind, string> = { add: '+', del: '-', context: ' ', hunk: '', meta: '' };
const srLabels: Partial<Record<DiffLineKind, string>> = { add: 'added: ', del: 'removed: ' };

export function DiffView({ diff }: { diff: string }) {
  const lines = useMemo(() => parseUnifiedDiff(diff), [diff]);
  if (lines.length === 0) {
    return <p className="text-sm text-gray-600 dark:text-gray-300">No textual changes.</p>;
  }
  return (
    <div className="overflow-x-auto rounded-md border border-gray-200 dark:border-gray-700">
      <table className="w-full border-collapse font-mono text-xs leading-5">
        <caption className="sr-only">Unified diff</caption>
        <tbody>
          {lines.map((line, i) => (
            <tr key={i} className={rowStyles[line.kind]} data-kind={line.kind}>
              <td className="w-12 select-none border-r border-gray-200 px-2 text-right text-gray-400 dark:border-gray-700">
                {line.oldLine ?? ''}
              </td>
              <td className="w-12 select-none border-r border-gray-200 px-2 text-right text-gray-400 dark:border-gray-700">
                {line.newLine ?? ''}
              </td>
              <td className="w-4 select-none pl-2" aria-hidden="true">
                {markers[line.kind]}
              </td>
              <td className="whitespace-pre-wrap break-all pr-3">
                {srLabels[line.kind] && <span className="sr-only">{srLabels[line.kind]}</span>}
                {line.text}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
