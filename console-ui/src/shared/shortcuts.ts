const MODIFIER_CODES = new Set([
  'ShiftLeft',
  'ShiftRight',
  'ControlLeft',
  'ControlRight',
  'AltLeft',
  'AltRight',
  'MetaLeft',
  'MetaRight',
]);

const MODIFIER_ALIASES: Record<string, string> = {
  meta: 'Meta',
  cmd: 'Meta',
  command: 'Meta',
  win: 'Meta',
  ctrl: 'Ctrl',
  control: 'Ctrl',
  alt: 'Alt',
  option: 'Alt',
  shift: 'Shift',
};

const MODIFIER_ORDER = ['Meta', 'Ctrl', 'Alt', 'Shift'];

export function eventToShortcut(event: KeyboardEvent): string | null {
  if (MODIFIER_CODES.has(event.code)) {
    return null;
  }
  const parts: string[] = [];
  if (event.metaKey) parts.push('Meta');
  if (event.ctrlKey) parts.push('Ctrl');
  if (event.altKey) parts.push('Alt');
  if (event.shiftKey) parts.push('Shift');
  parts.push(event.code);
  return parts.join('+');
}

export function normalizeShortcut(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (!trimmed.includes('+') && !looksLikeCode(trimmed)) {
    const code = keyToCode(trimmed);
    return code ? code : null;
  }
  const parts = trimmed.split('+').map((part) => part.trim()).filter(Boolean);
  if (parts.length === 0) return null;
  const modifiers = new Set<string>();
  let code: string | null = null;
  for (const part of parts) {
    const mapped = MODIFIER_ALIASES[part.toLowerCase()];
    if (mapped) {
      modifiers.add(mapped);
      continue;
    }
    code = part;
  }
  if (!code) return null;
  let normalizedCode = code;
  if (!looksLikeCode(normalizedCode)) {
    const mapped = keyToCode(normalizedCode);
    if (!mapped) return null;
    normalizedCode = mapped;
  }
  const ordered = MODIFIER_ORDER.filter((mod) => modifiers.has(mod));
  ordered.push(normalizedCode);
  return ordered.join('+');
}

export function formatShortcut(raw: string): string {
  const normalized = normalizeShortcut(raw);
  if (!normalized) return '';
  const parts = normalized.split('+');
  const code = parts.pop() ?? '';
  const displayParts = parts.map(formatModifier);
  displayParts.push(formatCode(code));
  return displayParts.join('+');
}

export function matchesShortcut(event: KeyboardEvent, raw: string): boolean {
  const wanted = normalizeShortcut(raw);
  if (!wanted) return false;
  const current = eventToShortcut(event);
  if (!current) return false;
  return normalizeShortcut(current) === wanted;
}

function formatModifier(mod: string): string {
  if (mod === 'Meta') return 'Cmd';
  if (mod === 'Ctrl') return 'Ctrl';
  if (mod === 'Alt') return 'Alt';
  if (mod === 'Shift') return 'Shift';
  return mod;
}

function formatCode(code: string): string {
  if (code.startsWith('Key')) return code.slice('Key'.length);
  if (code.startsWith('Digit')) return code.slice('Digit'.length);
  if (code.startsWith('Numpad')) return `Numpad${code.slice('Numpad'.length)}`;
  const map: Record<string, string> = {
    ArrowUp: 'Up',
    ArrowDown: 'Down',
    ArrowLeft: 'Left',
    ArrowRight: 'Right',
    Escape: 'Esc',
    Space: 'Space',
    Enter: 'Enter',
    Tab: 'Tab',
    Backspace: 'Backspace',
    Delete: 'Delete',
    Home: 'Home',
    End: 'End',
    PageUp: 'PageUp',
    PageDown: 'PageDown',
    Insert: 'Insert',
    Comma: ',',
  };
  return map[code] ?? code;
}

function looksLikeCode(value: string): boolean {
  return (
    /^Key[A-Z]$/.test(value) ||
    /^Digit[0-9]$/.test(value) ||
    /^Numpad[0-9]$/.test(value) ||
    /^F([1-9]|1[0-9]|2[0-4])$/.test(value) ||
    /^(ArrowUp|ArrowDown|ArrowLeft|ArrowRight)$/.test(value) ||
    /^(Tab|Escape|Space|Enter|Backspace|Delete|Home|End|PageUp|PageDown|Insert|Comma)$/.test(value)
  );
}

function keyToCode(key: string): string | null {
  if (looksLikeCode(key)) {
    return key;
  }
  if (key.length === 1) {
    const upper = key.toUpperCase();
    if (upper >= 'A' && upper <= 'Z') {
      return `Key${upper}`;
    }
    if (upper >= '0' && upper <= '9') {
      return `Digit${upper}`;
    }
  }
  const normalized = key.toLowerCase();
  const named: Record<string, string> = {
    tab: 'Tab',
    escape: 'Escape',
    esc: 'Escape',
    space: 'Space',
    enter: 'Enter',
    backspace: 'Backspace',
    delete: 'Delete',
    home: 'Home',
    end: 'End',
    pageup: 'PageUp',
    pagedown: 'PageDown',
    insert: 'Insert',
    arrowup: 'ArrowUp',
    arrowdown: 'ArrowDown',
    arrowleft: 'ArrowLeft',
    arrowright: 'ArrowRight',
    ',': 'Comma',
  };
  return named[normalized] ?? null;
}
