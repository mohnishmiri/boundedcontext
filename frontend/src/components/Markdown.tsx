import ReactMarkdown, { type Components } from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import remarkGfm from 'remark-gfm';

import { MermaidDiagram } from './MermaidDiagram';

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
};

/**
 * Renders untrusted Markdown produced by the agent. Raw HTML is never interpreted (react-markdown shows it as
 * text) and the resulting tree is sanitised with rehype-sanitize's GitHub-like schema.
 */
export function Markdown({ source }: { source: string }) {
  return (
    <div className="prose prose-sm max-w-none dark:prose-invert sm:prose-base">
      <ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeSanitize]} components={components}>
        {source}
      </ReactMarkdown>
    </div>
  );
}
