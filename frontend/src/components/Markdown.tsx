import { memo, useRef, type ComponentPropsWithoutRef } from 'react';
import ReactMarkdown, { type Components } from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import remarkGfm from 'remark-gfm';

import { downloadXlsx, exportFileName, nearestHeading, tableRows } from '../lib/xlsx';
import { MermaidDiagram } from './MermaidDiagram';

const remarkPlugins = [remarkGfm];
const rehypePlugins = [rehypeSanitize];

function DocTable({ children, ...props }: ComponentPropsWithoutRef<'table'>) {
  const wrapper = useRef<HTMLDivElement>(null);
  const table = useRef<HTMLTableElement>(null);
  const exportTable = () => {
    if (!table.current) return;
    const name = nearestHeading(wrapper.current) || 'Table';
    downloadXlsx(exportFileName(name), [{ name, rows: tableRows(table.current) }]);
  };
  return (
    <div ref={wrapper} className="my-6">
      <div className="not-prose mb-1 flex justify-end">
        <button
          type="button"
          onClick={exportTable}
          className="rounded px-1.5 py-0.5 text-xs font-medium text-indigo-600 hover:bg-indigo-50 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500 dark:text-indigo-400 dark:hover:bg-gray-800"
        >
          Export to Excel
        </button>
      </div>
      <div className="overflow-x-auto rounded-md border border-gray-200 dark:border-gray-700">
        <table ref={table} {...props} className="my-0 min-w-full">
          {children}
        </table>
      </div>
    </div>
  );
}

function hasClass(value: unknown, cls: string): boolean {
  if (Array.isArray(value)) return value.includes(cls);
  if (typeof value === 'string') return value.split(/\s+/).includes(cls);
  return false;
}

const components: Components = {
  pre({ node, children, ...props }) {
    const first = node?.children[0];
    if (first && first.type === 'element' && first.tagName === 'code' && hasClass(first.properties?.className, 'language-mermaid')) {
      return <>{children}</>;
    }
    return <pre {...props}>{children}</pre>;
  },
  code({ node: _node, className, children, ...props }) {
    if (hasClass(className, 'language-mermaid')) {
      return <MermaidDiagram code={String(children).replace(/\n$/, '')} />;
    }
    return (
      <code className={className} {...props}>
        {children}
      </code>
    );
  },
  a({ node: _node, href, children, ...props }) {
    const external = typeof href === 'string' && /^https?:\/\//i.test(href);
    return (
      <a href={href} {...props} {...(external ? { target: '_blank', rel: 'noopener noreferrer nofollow' } : {})}>
        {children}
      </a>
    );
  },
  // Wide tables scroll inside the document instead of overflowing the page.
  table({ node: _node, ref: _ref, ...props }) {
    return <DocTable {...props} />;
  },
};

/**
 * Renders untrusted Markdown produced by the agent. Raw HTML is never interpreted (react-markdown shows it as
 * text) and the resulting tree is sanitised with rehype-sanitize's GitHub-like schema. Memoised: parsing a large
 * document is the most expensive render on the job page.
 */
export const Markdown = memo(function Markdown({ source }: { source: string }) {
  return (
    <div
      className={
        'prose prose-sm min-w-0 max-w-none break-words dark:prose-invert sm:prose-base ' +
        // Evidence cells hold long unbroken paths (pkg/Class.java#method()); let them wrap anywhere.
        '[&_td]:align-top [&_td]:[overflow-wrap:anywhere] [&_th]:whitespace-nowrap [&_th]:bg-gray-50 ' +
        'dark:[&_th]:bg-gray-800 [&_td]:px-3 [&_th]:px-3 [&_code]:[overflow-wrap:anywhere]'
      }
    >
      <ReactMarkdown remarkPlugins={remarkPlugins} rehypePlugins={rehypePlugins} components={components}>
        {source}
      </ReactMarkdown>
    </div>
  );
});
