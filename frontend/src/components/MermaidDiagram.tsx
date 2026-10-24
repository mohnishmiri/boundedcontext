import { useEffect, useId, useRef, useState } from 'react';

type MermaidApi = typeof import('mermaid').default;

let mermaidPromise: Promise<MermaidApi> | null = null;
let renderCounter = 0;

function loadMermaid(): Promise<MermaidApi> {
  mermaidPromise ??= import('mermaid').then(({ default: mermaid }) => {
    const dark = typeof window.matchMedia === 'function' && window.matchMedia('(prefers-color-scheme: dark)').matches;
    // strict: labels are sanitised, HTML labels and click handlers are disabled.
    mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: dark ? 'dark' : 'default' });
    return mermaid;
  });
  return mermaidPromise;
}

/**
 * Renders a Mermaid diagram from agent output. The SVG produced by Mermaid in `strict` security mode is the only
 * agent-derived markup inserted as HTML, and only into this dedicated container.
 */
export function MermaidDiagram({ code }: { code: string }) {
  const container = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [rendered, setRendered] = useState(false);
  const baseId = useId().replace(/[^a-zA-Z0-9_-]/g, '');

  useEffect(() => {
    let cancelled = false;
    const renderId = `mermaid-${baseId}-${++renderCounter}`;
    setError(null);
    setRendered(false);
    loadMermaid()
      .then((mermaid) => mermaid.render(renderId, code))
      .then(({ svg }) => {
        if (cancelled || !container.current) return;
        container.current.innerHTML = svg;
        setRendered(true);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : 'Invalid diagram');
      })
      .finally(() => {
        // Mermaid leaves its scratch element behind when parsing fails.
        document.getElementById(`d${renderId}`)?.remove();
      });
    return () => {
      cancelled = true;
    };
  }, [code, baseId]);

  if (error) {
    return (
      <div className="not-prose my-4 rounded-md border border-amber-300 p-3 dark:border-amber-800">
        <p className="mb-2 text-xs text-amber-800 dark:text-amber-200">Diagram could not be rendered; showing source.</p>
        <pre className="overflow-x-auto text-xs">
          <code>{code}</code>
        </pre>
      </div>
    );
  }
  return (
    <div className="not-prose my-4 overflow-x-auto">
      {!rendered && <p className="text-xs text-gray-500">Rendering diagram…</p>}
      <div ref={container} role="img" aria-label="Diagram" className="[&_svg]:mx-auto [&_svg]:max-w-full" />
    </div>
  );
}
