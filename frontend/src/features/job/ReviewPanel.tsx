import { useState } from 'react';

import { jobOutputUrl } from '../../api/client';
import { useJobDiff } from '../../api/queries';
import type { Job } from '../../api/types';
import { Alert } from '../../components/Alert';
import { DiffView } from '../../components/DiffView';
import { ErrorPanel } from '../../components/ErrorPanel';
import { Markdown } from '../../components/Markdown';
import { Spinner } from '../../components/Spinner';
import { formatBytes, shortSha } from '../../lib/format';
import { buttonClass } from '../../lib/styles';

type Tab = 'preview' | 'diff';

const isMarkdown = (path: string) => /\.(md|markdown|mdx)$/i.test(path);

export function ReviewPanel({ job }: { job: Job }) {
  const diff = useJobDiff(job.id, job.status === 'succeeded');
  const [tab, setTab] = useState<Tab>('preview');
  const [selectedPath, setSelectedPath] = useState<string | null>(null);

  const primary = job.output?.primary_path;
  const files = [...(diff.data?.files ?? [])].sort((a, b) =>
    a.path === primary ? -1 : b.path === primary ? 1 : a.path.localeCompare(b.path),
  );
  const file = files.find((f) => f.path === selectedPath) ?? files[0];
  const unexpected = diff.data?.unexpected_changes ?? job.output?.unexpected_changes ?? [];

  return (
    <section aria-labelledby="review-heading" className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 id="review-heading" className="text-lg font-semibold">
          Review generated documentation
        </h2>
        <a href={jobOutputUrl(job.id)} download className={buttonClass('secondary')}>
          Download {primary ? primary.split('/').pop() : 'output'}
        </a>
      </div>

      {diff.isPending && <Spinner label="Loading generated files…" />}
      {diff.isError && <ErrorPanel error={diff.error} title="Could not load the generated files" />}

      {files.length > 1 && (
        <div role="group" aria-label="Generated files" className="flex flex-wrap gap-2">
          {files.map((f) => (
            <button
              key={f.path}
              type="button"
              aria-pressed={f.path === file?.path}
              onClick={() => setSelectedPath(f.path)}
              className={`rounded-md px-2.5 py-1 font-mono text-xs ring-1 ring-inset focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500 ${
                f.path === file?.path
                  ? 'bg-indigo-600 text-white ring-indigo-600'
                  : 'ring-gray-300 hover:bg-gray-50 dark:ring-gray-600 dark:hover:bg-gray-800'
              }`}
            >
              {f.path}
            </button>
          ))}
        </div>
      )}

      {file && (
        <div className="space-y-3">
          <p className="text-sm text-gray-600 dark:text-gray-300">
            <code className="font-mono">{file.path}</code> · {file.change_type === 'added' ? 'new file' : 'modified'} ·{' '}
            {formatBytes(file.size_bytes)} · base <span className="font-mono">{shortSha(diff.data?.base_sha)}</span>
          </p>
          <div role="tablist" aria-label="View" className="flex gap-1 border-b border-gray-200 dark:border-gray-700">
            {(['preview', 'diff'] as const).map((t) => (
              <button
                key={t}
                id={`tab-${t}`}
                type="button"
                role="tab"
                aria-selected={tab === t}
                aria-controls={`panel-${t}`}
                tabIndex={tab === t ? 0 : -1}
                onClick={() => setTab(t)}
                onKeyDown={(e) => {
                  if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
                    const next = t === 'preview' ? 'diff' : 'preview';
                    setTab(next);
                    document.getElementById(`tab-${next}`)?.focus();
                  }
                }}
                className={`-mb-px border-b-2 px-3 py-2 text-sm font-medium focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500 ${
                  tab === t
                    ? 'border-indigo-600 text-indigo-700 dark:text-indigo-300'
                    : 'border-transparent text-gray-600 hover:text-gray-900 dark:text-gray-300 dark:hover:text-white'
                }`}
              >
                {t === 'preview' ? 'Preview' : 'Diff'}
              </button>
            ))}
          </div>
          <div
            id={`panel-${tab}`}
            role="tabpanel"
            aria-labelledby={`tab-${tab}`}
            className="rounded-md border border-gray-200 p-4 dark:border-gray-700"
          >
            {tab === 'preview' ? (
              isMarkdown(file.path) ? (
                <Markdown source={file.new_content} />
              ) : (
                <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs">{file.new_content}</pre>
              )
            ) : (
              <DiffView diff={file.unified_diff} />
            )}
          </div>
        </div>
      )}

      {diff.data && files.length === 0 && (
        <Alert tone="warning">The job succeeded but no generated files were recorded.</Alert>
      )}

      {unexpected.length > 0 && (
        <Alert tone="warning" title="Other changes made by the agent — never committed">
          <p>
            The agent also changed these files outside its allowed output paths. They are discarded and will not be
            part of the pull request.
          </p>
          <ul className="list-inside list-disc font-mono text-xs">
            {unexpected.map((c) => (
              <li key={c.path}>
                {c.path} <span className="font-sans">({c.change_type})</span>
              </li>
            ))}
          </ul>
        </Alert>
      )}
    </section>
  );
}
