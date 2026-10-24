export type DiffLineKind = 'add' | 'del' | 'context' | 'hunk' | 'meta';

export type DiffLine = {
  kind: DiffLineKind;
  text: string;
  oldLine: number | null;
  newLine: number | null;
};

const HUNK_RE = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

/** Parses a unified diff into display lines with old/new line numbers. */
export function parseUnifiedDiff(diff: string): DiffLine[] {
  const lines = diff.replace(/\r\n/g, '\n').split('\n');
  if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop();
  const out: DiffLine[] = [];
  let oldNo = 0;
  let newNo = 0;
  let inHunk = false;
  for (const line of lines) {
    const hunk = HUNK_RE.exec(line);
    if (hunk) {
      oldNo = Number(hunk[1]);
      newNo = Number(hunk[2]);
      inHunk = true;
      out.push({ kind: 'hunk', text: line, oldLine: null, newLine: null });
      continue;
    }
    if (!inHunk || line.startsWith('\\')) {
      out.push({ kind: 'meta', text: line, oldLine: null, newLine: null });
      continue;
    }
    if (line.startsWith('+')) {
      out.push({ kind: 'add', text: line.slice(1), oldLine: null, newLine: newNo++ });
    } else if (line.startsWith('-')) {
      out.push({ kind: 'del', text: line.slice(1), oldLine: oldNo++, newLine: null });
    } else {
      out.push({ kind: 'context', text: line.startsWith(' ') ? line.slice(1) : line, oldLine: oldNo++, newLine: newNo++ });
    }
  }
  return out;
}
