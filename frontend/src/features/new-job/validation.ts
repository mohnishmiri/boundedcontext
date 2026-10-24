import type { AgentInput } from '../../api/types';

function hasControlChars(value: string): boolean {
  for (const ch of value) {
    const c = ch.codePointAt(0) ?? 0;
    if (c < 0x20 || c === 0x7f || (c >= 0x80 && c < 0xa0)) return true;
  }
  return false;
}

/** Client-side mirror of the server's repository-relative path rule; the server remains authoritative. */
export function validateRelativePath(value: string): string | null {
  const msg = "Must be a relative path inside the repository (no '..', no leading '/')";
  if (value.startsWith('/') || value.includes('\\') || value.includes(':') || hasControlChars(value)) {
    return msg;
  }
  const parts = value.split('/');
  if (parts.some((p) => p === '' || p === '.' || p === '..')) return msg;
  if (parts[0].toLowerCase() === '.git') return msg;
  return null;
}

export function validateInput(input: AgentInput, value: string): string | null {
  if (input.required && value.trim() === '') return 'This field is required';
  if (value === '') return null;
  if ([...value].length > input.max_length) return `Must be at most ${input.max_length} characters`;
  if (hasControlChars(value)) return 'Must not contain control characters or line breaks';
  if (input.kind === 'path') return validateRelativePath(value);
  if (input.kind === 'select' && input.choices && !input.choices.includes(value)) return 'Choose one of the options';
  return null;
}

/** Server field errors may be keyed by the input name or by `inputs.<name>`. */
export function inputFieldError(errors: Record<string, string>, name: string): string | undefined {
  return errors[name] ?? errors[`inputs.${name}`];
}
