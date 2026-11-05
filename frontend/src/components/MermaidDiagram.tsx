import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type PointerEvent } from 'react';

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

const MIN_ZOOM = 0.1;
const MAX_ZOOM = 8;
const ZOOM_STEP = 1.25;
const clampZoom = (z: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));

type Size = { width: number; height: number };

function naturalSize(svg: SVGSVGElement): Size | null {
  const vb = svg.getAttribute('viewBox')?.trim().split(/[\s,]+/).map(Number);
  if (vb && vb.length === 4 && vb[2] > 0 && vb[3] > 0) return { width: vb[2], height: vb[3] };
  const w = Number.parseFloat(svg.getAttribute('width') ?? '');
  const h = Number.parseFloat(svg.getAttribute('height') ?? '');
  return w > 0 && h > 0 ? { width: w, height: h } : null;
}

const toolButton =
  'inline-flex h-7 min-w-7 items-center justify-center rounded px-1.5 text-xs font-medium text-gray-700 ring-1 ring-inset ' +
  'ring-gray-300 hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500 ' +
  'disabled:opacity-40 dark:text-gray-200 dark:ring-gray-600 dark:hover:bg-gray-800';

/**
 * Renders a Mermaid diagram from agent output. The SVG produced by Mermaid in `strict` security mode is the only
 * agent-derived markup inserted as HTML, and only into this dedicated container. Diagrams can be zoomed (buttons
 * or Ctrl + wheel), panned by dragging, opened full screen and downloaded as SVG.
 */
export function MermaidDiagram({ code }: { code: string }) {
  const container = useRef<HTMLDivElement>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const toggleButton = useRef<HTMLButtonElement>(null);
  const natural = useRef<Size | null>(null);
  const drag = useRef<{ x: number; y: number; left: number; top: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [rendered, setRendered] = useState(false);
  /** `null` fits the diagram to the available space. */
  const [zoom, setZoom] = useState<number | null>(null);
  const [expanded, setExpanded] = useState(false);
  // Only re-renders on resize so that "fit" is recomputed.
  const [, setViewportWidth] = useState(0);
  const baseId = useId().replace(/[^a-zA-Z0-9_-]/g, '');

  useEffect(() => {
    let cancelled = false;
    const renderId = `mermaid-${baseId}-${++renderCounter}`;
    setError(null);
    setRendered(false);
    setZoom(null);
    loadMermaid()
      .then((mermaid) => mermaid.render(renderId, code))
      .then(({ svg }) => {
        if (cancelled || !container.current) return;
        container.current.innerHTML = svg;
        const el = container.current.querySelector('svg');
        natural.current = el ? naturalSize(el) : null;
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

  // "Fit" follows window resizes and the full-screen toggle.
  useEffect(() => {
    const el = viewport.current;
    if (!el || !rendered) return;
    setViewportWidth(el.clientWidth);
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => setViewportWidth(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, [rendered, expanded]);

  const fitScale = useCallback((): number => {
    const size = natural.current;
    const el = viewport.current;
    if (!size || !el || el.clientWidth === 0) return 1;
    const byWidth = (el.clientWidth - 16) / size.width;
    // Inline: never enlarge past natural size. Full screen: show the whole diagram, enlarging small ones.
    return expanded ? Math.min(byWidth, (el.clientHeight - 16) / size.height) : Math.min(1, byWidth);
  }, [expanded]);

  const scale = rendered && natural.current ? clampZoom(zoom ?? fitScale()) : 1;

  useLayoutEffect(() => {
    const svg = container.current?.querySelector('svg');
    const size = natural.current;
    if (!svg || !size || !rendered) return;
    svg.style.maxWidth = 'none';
    svg.style.width = `${Math.round(size.width * scale)}px`;
    svg.style.height = `${Math.round(size.height * scale)}px`;
  }, [scale, rendered]);

  // Ctrl/Cmd + wheel zooms; needs a non-passive listener to stop the browser's own page zoom.
  useEffect(() => {
    const el = viewport.current;
    if (!el || !rendered) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      setZoom((z) => clampZoom((z ?? fitScale()) * (e.deltaY < 0 ? 1.1 : 1 / 1.1)));
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, [rendered, fitScale]);

  useEffect(() => {
    if (!expanded) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    toggleButton.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setExpanded(false);
        setZoom(null);
      }
    };
    document.addEventListener('keydown', onKey);
    return () => {
      document.body.style.overflow = previousOverflow;
      document.removeEventListener('keydown', onKey);
    };
  }, [expanded]);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    const el = viewport.current;
    if (!el || e.button !== 0 || (el.scrollWidth <= el.clientWidth && el.scrollHeight <= el.clientHeight)) return;
    drag.current = { x: e.clientX, y: e.clientY, left: el.scrollLeft, top: el.scrollTop };
    el.setPointerCapture?.(e.pointerId);
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const el = viewport.current;
    if (!el || !drag.current) return;
    el.scrollLeft = drag.current.left - (e.clientX - drag.current.x);
    el.scrollTop = drag.current.top - (e.clientY - drag.current.y);
  };
  const endDrag = () => {
    drag.current = null;
  };

  const downloadSvg = () => {
    const svg = container.current?.querySelector('svg');
    if (!svg) return;
    const copy = svg.cloneNode(true) as SVGSVGElement;
    copy.removeAttribute('style');
    if (natural.current) {
      copy.setAttribute('width', String(natural.current.width));
      copy.setAttribute('height', String(natural.current.height));
    }
    const url = URL.createObjectURL(new Blob([copy.outerHTML], { type: 'image/svg+xml' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = 'diagram.svg';
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 10_000);
  };

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

  const zoomable = rendered && natural.current !== null;
  const pannable = zoom !== null || expanded;
  return (
    <div
      className={
        expanded
          ? 'not-prose fixed inset-0 z-50 flex flex-col gap-2 bg-white p-4 dark:bg-gray-950'
          : 'not-prose my-4 rounded-md border border-gray-200 dark:border-gray-700'
      }
      {...(expanded ? { role: 'dialog', 'aria-modal': true, 'aria-label': 'Diagram (full screen)' } : {})}
    >
      {rendered && (
        <div
          role="toolbar"
          aria-label="Diagram controls"
          className={`flex flex-wrap items-center justify-end gap-1 ${
            expanded ? '' : 'border-b border-gray-200 px-2 py-1 dark:border-gray-700'
          }`}
        >
          {zoomable && (
            <>
              <button
                type="button"
                className={toolButton}
                aria-label="Zoom out"
                title="Zoom out (Ctrl + wheel)"
                disabled={scale <= MIN_ZOOM}
                onClick={() => setZoom(clampZoom(scale / ZOOM_STEP))}
              >
                −
              </button>
              <span className="w-12 text-center text-xs tabular-nums text-gray-600 dark:text-gray-300">
                {Math.round(scale * 100)}%
              </span>
              <button
                type="button"
                className={toolButton}
                aria-label="Zoom in"
                title="Zoom in (Ctrl + wheel)"
                disabled={scale >= MAX_ZOOM}
                onClick={() => setZoom(clampZoom(scale * ZOOM_STEP))}
              >
                +
              </button>
              <button type="button" className={toolButton} onClick={() => setZoom(1)} title="Actual size">
                100%
              </button>
              <button type="button" className={toolButton} onClick={() => setZoom(null)} title="Fit to view">
                Fit
              </button>
            </>
          )}
          <button type="button" className={toolButton} onClick={downloadSvg} title="Download as SVG">
            SVG
          </button>
          <button
            ref={toggleButton}
            type="button"
            className={toolButton}
            aria-pressed={expanded}
            onClick={() => {
              setExpanded((x) => !x);
              setZoom(null);
            }}
          >
            {expanded ? 'Close (Esc)' : 'Full screen'}
          </button>
        </div>
      )}
      <div
        ref={viewport}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        className={`${expanded ? 'min-h-0 flex-1' : pannable ? 'max-h-[75vh]' : ''} overflow-auto p-2 ${
          pannable ? 'cursor-grab active:cursor-grabbing' : ''
        }`}
      >
        {!rendered && <p className="text-xs text-gray-500">Rendering diagram…</p>}
        <div ref={container} role="img" aria-label="Diagram" className="w-max min-w-full [&_svg]:mx-auto [&_svg]:block" />
      </div>
    </div>
  );
}
