import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Markdown } from './Markdown';

const renderMock = vi.fn();
vi.mock('mermaid', () => ({
  default: {
    initialize: vi.fn(),
    render: (...args: unknown[]) => renderMock(...args),
  },
}));

describe('Markdown preview', () => {
  it('renders GitHub-flavoured Markdown', () => {
    render(<Markdown source={'# Contexts\n\n| a | b |\n|---|---|\n| 1 | 2 |\n'} />);
    expect(screen.getByRole('heading', { name: 'Contexts' })).toBeInTheDocument();
    expect(screen.getByRole('table')).toBeInTheDocument();
  });

  it('never renders raw HTML, event handlers or javascript: links from agent output', () => {
    const source = [
      '<script>window.__pwned = true</script>',
      '',
      '<img src="x" onerror="window.__pwned = true">',
      '',
      '[click me](javascript:alert(1))',
      '',
      '<a href="https://evil.example" onclick="steal()">x</a>',
    ].join('\n');
    const { container } = render(<Markdown source={source} />);
    expect(container.querySelector('script')).toBeNull();
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('[onerror]')).toBeNull();
    expect(container.querySelector('[onclick]')).toBeNull();
    for (const a of container.querySelectorAll('a')) {
      expect(a.getAttribute('href') ?? '').not.toMatch(/^javascript:/i);
    }
    expect((window as unknown as { __pwned?: boolean }).__pwned).toBeUndefined();
  });

  it('renders mermaid blocks with the mermaid renderer', async () => {
    renderMock.mockResolvedValueOnce({ svg: '<svg data-testid="diagram"><g></g></svg>' });
    render(<Markdown source={'```mermaid\nflowchart LR\n  A --> B\n```\n'} />);
    await waitFor(() => expect(screen.getByTestId('diagram')).toBeInTheDocument());
    expect(renderMock).toHaveBeenCalledWith(expect.stringMatching(/^mermaid-/), 'flowchart LR\n  A --> B');
  });

  it('falls back to the diagram source when mermaid cannot parse it', async () => {
    renderMock.mockRejectedValueOnce(new Error('Parse error'));
    render(<Markdown source={'```mermaid\nnot a diagram\n```\n'} />);
    expect(await screen.findByText(/Diagram could not be rendered/)).toBeInTheDocument();
    expect(screen.getByText('not a diagram')).toBeInTheDocument();
  });
});
