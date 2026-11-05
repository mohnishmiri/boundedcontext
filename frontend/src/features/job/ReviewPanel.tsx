import { useEffect, useRef, useState } from 'react';

import { jobOutputUrl } from '../../api/client';
import { useJobDiff } from '../../api/queries';
import type { Job } from '../../api/types';
import { Alert } from '../../components/Alert';
import { DiffView } from '../../components/DiffView';
import { ErrorPanel } from '../../components/ErrorPanel';
import { ExportButton } from '../../components/ExportButton';
import { Markdown } from '../../components/Markdown';
import { Spinner } from '../../components/Spinner';
import { formatBytes, shortSha } from '../../lib/format';
import { buttonClass, inputClass } from '../../lib/styles';
import { nearestHeading, tableRows, type Sheet } from '../../lib/xlsx';

type Tab = 'preview' | 'diff';

const FILES_OPEN_KEY = 'bcp.review.filesOpen';

function readFilesOpen(): boolean {
  try {
    return window.localStorage.getItem(FILES_OPEN_KEY) !== 'false';
  } catch {
    return true;
  }
}

/** Every table of the rendered document as one sheet each, named after the heading above it. */
function documentTables(panel: HTMLElement | null): Sheet[] {
  const tables = Array.from(panel?.querySelectorAll('table') ?? []);
  if (tables.length === 0) throw new Error('This document has no tables.');
  return tables.map((t, i) => ({
    name: nearestHeading(t.parentElement?.parentElement ?? null) || `Table ${i + 1}`,
    rows: tableRows(t),
  }));
}

const isMarkdown = (path: string) => /\.(md|markdown|mdx)$/i.test(path);

/** Empty-folder placeholders; jobs from before the worker ignored them still list them. */
const isPlaceholder = (path: string) => /(^|\/)\.(git)?keep$/.test(path);

const FILE_SEARCH_MIN = 6;

function splitPath(path: string): { dir: string; name: string } {
  const i = path.lastIndexOf('/');
  return i < 0 ? { dir: '', name: path } : { dir: path.slice(0, i + 1), name: path.slice(i + 1) };
}

function FileIcon({ markdown }: { markdown: boolean }) {
  return (
    <svg viewBox="0 0 16 16" className="mt-0.5 h-4 w-4 shrink-0 opacity-70" fill="currentColor" aria-hidden="true">
      {markdown ? (
        <path d="M14.85 3H1.15C.52 3 0 3.52 0 4.15v7.69C0 12.48.52 13 1.15 13h13.69c.64 0 1.15-.52 1.15-1.15v-7.7C16 3.52 15.48 3 14.85 3ZM9 11H7V8L5.5 9.92 4 8v3H2V5h2l1.5 2L7 5h2v6Zm2.99.5L9.5 8H11V5h2v3h1.5l-2.51 3.5Z" />
      ) : (
        <path d="M2 1.75C2 .784 2.784 0 3.75 0h6.586c.464 0 .909.184 1.237.513l2.914 2.914c.329.328.513.773.513 1.237v9.586A1.75 1.75 0 0 1 13.25 16h-9.5A1.75 1.75 0 0 1 2 14.25Zm1.75-.25a.25.25 0 0 0-.25.25v12.5c0 .138.112.25.25.25h9.5a.25.25 0 0 0 .25-.25V6h-2.75A1.75 1.75 0 0 1 9 4.25V1.5Zm6.75.062V4.25c0 .138.112.25.25.25h2.688l-.011-.013-2.914-2.914-.013-.011Z" />
      )}
    </svg>
  );
}

export function ReviewPanel({ job }: { job: Job }) {
  const diff = useJobDiff(job.id, job.status === 'succeeded');
  const [tab, setTab] = useState<Tab>('preview');
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [fileQuery, setFileQuery] = useState('');
  const [filesOpen, setFilesOpen] = useState(readFilesOpen);
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    try {
      window.localStorage.setItem(FILES_OPEN_KEY, String(filesOpen));
    } catch {
      // Storage disabled: the choice just is not remembered.
    }
  }, [filesOpen]);

  const primary = job.output?.primary_path;
  const files = [...(diff.data?.files ?? [])].sort((a, b) =>
    a.path === primary ? -1 : b.path === primary ? 1 : a.path.localeCompare(b.path),
  );
  const file = files.find((f) => f.path === selectedPath) ?? files[0];
  const needle = fileQuery.trim().toLowerCase();
  const listed = needle ? files.filter((f) => f.path.toLowerCase().includes(needle)) : files;
  const unexpected = (diff.data?.unexpected_changes ?? job.output?.unexpected_changes ?? []).filter(
    (c) => !isPlaceholder(c.path),
  );
  const totalBytes = files.reduce((sum, f) => sum + f.size_bytes, 0);

  return (
    <section aria-labelledby="review-heading" className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 id="review-heading" className="text-lg font-semibold">
            Review generated documentation
          </h2>
          {files.length > 0 && (
            <p className="text-sm text-gray-600 dark:text-gray-300">
              {files.length} file{files.length === 1 ? '' : 's'} · {formatBytes(totalBytes)} · base{' '}
              <span className="font-mono">{shortSha(diff.data?.base_sha)}</span>
            </p>
          )}
        </div>
        <a href={jobOutputUrl(job.id)} download className={buttonClass('secondary')}>
          Download {primary ? primary.split('/').pop() : 'output'}
        </a>
      </div>

      {diff.isPending && <Spinner label="Loading generated files…" />}
      {diff.isError && <ErrorPanel error={diff.error} title="Could not load the generated files" />}

      {file && (
        <div className={files.length > 1 && filesOpen ? 'grid gap-4 xl:grid-cols-[15rem_minmax(0,1fr)]' : ''}>
          {files.length > 1 && filesOpen && (
            <nav aria-label="Generated files" className="hidden min-w-0 space-y-2 xl:block">
              {files.length >= FILE_SEARCH_MIN && (
                <input
                  type="search"
                  aria-label="Search generated files"
                  placeholder={`Search ${files.length} files`}
                  value={fileQuery}
                  onChange={(e) => setFileQuery(e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                  className={inputClass}
                />
              )}
              {listed.length === 0 && (
                <p className="px-2 text-sm text-gray-600 dark:text-gray-300">No file matches “{fileQuery.trim()}”.</p>
              )}
              <ul className="max-h-[70vh] space-y-1 overflow-y-auto rounded-md border border-gray-200 p-1.5 lg:sticky lg:top-4 dark:border-gray-700">
                {listed.map((f) => {
                  const { dir, name } = splitPath(f.path);
                  const selected = f.path === file.path;
                  return (
                    <li key={f.path}>
                      <button
                        type="button"
                        aria-pressed={selected}
                        aria-label={f.path}
                        title={f.path}
                        onClick={() => setSelectedPath(f.path)}
                        className={`flex w-full items-start gap-2 rounded px-2 py-1.5 text-left text-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500 ${
                          selected
                            ? 'bg-indigo-50 text-indigo-900 ring-1 ring-inset ring-indigo-200 dark:bg-indigo-950 dark:text-indigo-100 dark:ring-indigo-800'
                            : 'text-gray-700 hover:bg-gray-50 dark:text-gray-200 dark:hover:bg-gray-800'
                        }`}
                      >
                        <FileIcon markdown={isMarkdown(f.path)} />
                        <span className="min-w-0">
                          <span className="block break-all font-medium">{name}</span>
                          {dir && <span className="block break-all text-xs text-gray-500 dark:text-gray-400">{dir}</span>}
                        </span>
                        {f.path === primary && (
                          <span className="ml-auto shrink-0 rounded bg-indigo-100 px-1.5 py-0.5 text-[10px] font-semibold uppercase text-indigo-700 dark:bg-indigo-900 dark:text-indigo-200">
                            Main
                          </span>
                        )}
                      </button>
                    </li>
                  );
                })}
              </ul>
            </nav>
          )}

          <div className="min-w-0 space-y-3">
            {files.length > 1 && (
              // Narrower screens (or a hidden file list) pick the file here so the preview keeps the full width.
              <div className={filesOpen ? 'xl:hidden' : ''}>
                <label htmlFor="generated-file" className="sr-only">
                  Generated file
                </label>
                <select
                  id="generated-file"
                  value={file.path}
                  onChange={(e) => setSelectedPath(e.target.value)}
                  className={inputClass}
                >
                  {files.map((f) => (
                    <option key={f.path} value={f.path}>
                      {f.path}
                      {f.path === primary ? ' (main)' : ''}
                    </option>
                  ))}
                </select>
              </div>
            )}
            <div className="flex flex-wrap items-end justify-between gap-2 border-b border-gray-200 dark:border-gray-700">
              <div className="flex items-end gap-2">
                {files.length > 1 && (
                  <button
                    type="button"
                    aria-expanded={filesOpen}
                    onClick={() => setFilesOpen((o) => !o)}
                    title={filesOpen ? 'Hide the file list to widen the preview' : 'Show the file list'}
                    className="mb-1 hidden items-center gap-1 rounded px-2 py-1 text-xs font-medium text-gray-600 ring-1 ring-inset ring-gray-300 hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500 xl:inline-flex dark:text-gray-300 dark:ring-gray-600 dark:hover:bg-gray-800"
                  >
                    <svg viewBox="0 0 20 20" className="h-3.5 w-3.5" fill="currentColor" aria-hidden="true">
                      <path
                        d={filesOpen ? 'M12.5 4.5 7 10l5.5 5.5-1.06 1.06L4.88 10l6.56-6.56z' : 'M7.5 4.5 13 10l-5.5 5.5 1.06 1.06L15.12 10 8.56 3.44z'}
                      />
                    </svg>
                    {filesOpen ? 'Hide files' : `Files (${files.length})`}
                  </button>
                )}
                <div role="tablist" aria-label="View" className="flex gap-1">
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
              </div>
              <div className="flex flex-wrap items-center gap-3 pb-1">
                <p className="text-xs text-gray-500 dark:text-gray-400">
                  <code className="break-all font-mono">{file.path}</code> ·{' '}
                  {file.change_type === 'added' ? 'new file' : 'modified'} · {formatBytes(file.size_bytes)}
                </p>
                {tab === 'preview' && isMarkdown(file.path) && (
                  <ExportButton
                    label="Export tables"
                    fileBase={splitPath(file.path).name.replace(/\.[^.]+$/, '')}
                    build={() => documentTables(panel.current)}
                  />
                )}
              </div>
            </div>
            <div
              ref={panel}
              id={`panel-${tab}`}
              role="tabpanel"
              aria-labelledby={`tab-${tab}`}
              className="min-w-0 overflow-hidden rounded-md border border-gray-200 bg-white p-4 sm:p-6 dark:border-gray-700 dark:bg-gray-900"
            >
              {tab === 'preview' ? (
                isMarkdown(file.path) ? (
                  <Markdown source={file.new_content} />
                ) : (
                  <pre className="overflow-x-auto whitespace-pre-wrap break-words font-mono text-xs">{file.new_content}</pre>
                )
              ) : (
                <div className="overflow-x-auto">
                  <DiffView diff={file.unified_diff} />
                </div>
              )}
            </div>
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
