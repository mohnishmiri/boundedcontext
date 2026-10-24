import { useEffect, useRef, useState } from 'react';

import type { LogEntry, LogLevel } from '../../api/types';
import { formatTime } from '../../lib/format';

const levelStyles: Record<LogLevel, string> = {
  debug: 'text-gray-500',
  info: 'text-gray-800 dark:text-gray-200',
  warn: 'text-amber-700 dark:text-amber-300',
  error: 'text-red-700 dark:text-red-300',
};

export function LogViewer({ logs, autoOpen }: { logs: LogEntry[]; autoOpen: boolean }) {
  const [open, setOpen] = useState(autoOpen);
  const scroller = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);

  useEffect(() => {
    if (autoOpen) setOpen(true);
  }, [autoOpen]);

  useEffect(() => {
    const el = scroller.current;
    if (open && el && stickToBottom.current) el.scrollTop = el.scrollHeight;
  }, [logs, open]);

  return (
    <section aria-labelledby="logs-heading" className="space-y-2">
      <h2 id="logs-heading" className="text-base font-semibold">
        <button
          type="button"
          className="inline-flex items-center gap-2 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500"
          aria-expanded={open}
          aria-controls="job-logs"
          onClick={() => setOpen((o) => !o)}
        >
          <svg
            viewBox="0 0 20 20"
            className={`h-4 w-4 transition-transform ${open ? 'rotate-90' : ''}`}
            fill="currentColor"
            aria-hidden="true"
          >
            <path d="M7 5l6 5-6 5z" />
          </svg>
          Logs <span className="text-sm font-normal text-gray-500">({logs.length})</span>
        </button>
      </h2>
      {open && (
        <div
          id="job-logs"
          ref={scroller}
          role="log"
          tabIndex={0}
          aria-label="Job logs"
          onScroll={(e) => {
            const el = e.currentTarget;
            stickToBottom.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 16;
          }}
          className="max-h-96 overflow-auto rounded-md border border-gray-200 bg-gray-50 p-3 font-mono text-xs leading-5 dark:border-gray-700 dark:bg-gray-950"
        >
          {logs.length === 0 ? (
            <p className="text-gray-500">No log output yet.</p>
          ) : (
            logs.map((l) => (
              <div key={l.seq} className={`flex gap-3 ${levelStyles[l.level]}`}>
                <time className="shrink-0 text-gray-400" dateTime={l.ts}>
                  {formatTime(l.ts)}
                </time>
                <span className="w-16 shrink-0 uppercase text-gray-400">{l.level}</span>
                <span className="whitespace-pre-wrap break-words">{l.message}</span>
              </div>
            ))
          )}
        </div>
      )}
      <p className="text-xs text-gray-500 dark:text-gray-400">Secrets are redacted before logs are stored.</p>
    </section>
  );
}
