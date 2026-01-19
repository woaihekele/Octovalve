import type { ITheme } from '@xterm/xterm';
import type { ResolvedTheme } from './theme';

function resolveCssColorVar(name: string, fallback: string): string {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return fallback;
  }
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  if (!raw) {
    return fallback;
  }
  if (raw.startsWith('rgb') || raw.startsWith('#')) {
    return raw;
  }
  const normalized = raw.replace(/\s+/g, ', ');
  return `rgb(${normalized})`;
}

function applySurfaceBackground(theme: ITheme): ITheme {
  const background = resolveCssColorVar('--color-bg', theme.background ?? '#000000');
  return {
    ...theme,
    background,
    cursorAccent: background,
  };
}

function buildExtendedAnsi(overrides: Record<number, string>): string[] {
  const colors: string[] = [];
  for (const [index, value] of Object.entries(overrides)) {
    const parsed = Number(index);
    if (Number.isNaN(parsed) || parsed < 16 || parsed > 255) {
      continue;
    }
    colors[parsed - 16] = value;
  }
  return colors;
}

const EXTENDED_ANSI_LIGHT = buildExtendedAnsi({
  3: '#9a6700',
  5: '#8250df',
  9: '#cf222e',
  10: '#2da44e',
  11: '#bf8700',
  13: '#8250df',
  15: '#ffffff',
  16: '#24292f',
  21: '#0969da',
  27: '#0969da',
  34: '#1a7f37',
  45: '#1b7c83',
  51: '#3192aa',
  196: '#a40e26',
  226: '#bf8700',
  232: '#24292f',
});

const EXTENDED_ANSI_DARK = buildExtendedAnsi({
  3: '#d29922',
  5: '#bc8cff',
  9: '#ff7b72',
  10: '#3fb950',
  11: '#e3b341',
  13: '#bc8cff',
  15: '#f0f6fc',
  16: '#0d1117',
  21: '#1f6feb',
  27: '#58a6ff',
  34: '#3fb950',
  45: '#39c5cf',
  51: '#56d4dd',
  196: '#da3633',
  226: '#e3b341',
  232: '#161b22',
});

const EXTENDED_ANSI_ONE_DARK = buildExtendedAnsi({
  3: '#d19a66',
  5: '#c678dd',
  9: '#e06c75',
  10: '#98c379',
  11: '#e5c07b',
  13: '#c678dd',
  15: '#abb2bf',
  16: '#282c34',
  21: '#61afef',
  27: '#61afef',
  34: '#98c379',
  45: '#56b6c2',
  51: '#56b6c2',
  196: '#e06c75',
  226: '#e5c07b',
  232: '#21252b',
});

const GITHUB_LIGHT_THEME: ITheme = {
  background: '#ffffff',
  foreground: '#24292f',
  cursor: '#8250df',
  cursorAccent: '#ffffff',
  selectionBackground: 'rgba(130, 80, 223, 0.22)',
  selectionInactiveBackground: 'rgba(130, 80, 223, 0.16)',
  selectionForeground: '#24292f',
  black: '#24292f',
  red: '#cf222e',
  green: '#1a7f37',
  yellow: '#9a6700',
  blue: '#0969da',
  magenta: '#8250df',
  cyan: '#1b7c83',
  white: '#d0d7de',
  brightBlack: '#57606a',
  brightRed: '#a40e26',
  brightGreen: '#2da44e',
  brightYellow: '#bf8700',
  brightBlue: '#218bff',
  brightMagenta: '#a475f9',
  brightCyan: '#3192aa',
  brightWhite: '#ffffff',
  extendedAnsi: EXTENDED_ANSI_LIGHT,
};

const DARCULA_THEME: ITheme = {
  background: '#232323',
  foreground: '#A9B7C6',
  cursor: '#A9B7C6',
  cursorAccent: '#232323',
  selectionBackground: 'rgba(104, 151, 187, 0.35)',
  selectionInactiveBackground: 'rgba(104, 151, 187, 0.22)',
  selectionForeground: '#A9B7C6',
  black: '#000000',
  red: '#cc666e',
  green: '#6a8759',
  yellow: '#bbb529',
  blue: '#6897bb',
  magenta: '#9876aa',
  cyan: '#6d9cbe',
  white: '#a9b7c6',
  brightBlack: '#4e5254',
  brightRed: '#d46a6a',
  brightGreen: '#87af5f',
  brightYellow: '#d0d050',
  brightBlue: '#7aa6c2',
  brightMagenta: '#b294bb',
  brightCyan: '#83b7c9',
  brightWhite: '#ffffff',
};

const GITHUB_DARK_THEME: ITheme = {
  background: '#0d1117',
  foreground: '#c9d1d9',
  cursor: '#8957e5',
  cursorAccent: '#0d1117',
  selectionBackground: 'rgba(137, 87, 229, 0.35)',
  selectionInactiveBackground: 'rgba(137, 87, 229, 0.22)',
  selectionForeground: '#c9d1d9',
  black: '#484f58',
  red: '#ff7b72',
  green: '#3fb950',
  yellow: '#d29922',
  blue: '#58a6ff',
  magenta: '#bc8cff',
  cyan: '#39c5cf',
  white: '#b1bac4',
  brightBlack: '#6e7681',
  brightRed: '#ffa198',
  brightGreen: '#56d364',
  brightYellow: '#e3b341',
  brightBlue: '#79c0ff',
  brightMagenta: '#d2a8ff',
  brightCyan: '#56d4dd',
  brightWhite: '#f0f6fc',
  extendedAnsi: EXTENDED_ANSI_DARK,
};

const ONE_DARK_PRO_THEME: ITheme = {
  background: '#1f2329',
  foreground: '#abb2bf',
  cursor: '#528bff',
  cursorAccent: '#1f2329',
  selectionBackground: 'rgba(82, 139, 255, 0.35)',
  selectionInactiveBackground: 'rgba(82, 139, 255, 0.22)',
  selectionForeground: '#abb2bf',
  black: '#3f4451',
  red: '#e06c75',
  green: '#98c379',
  yellow: '#e5c07b',
  blue: '#61afef',
  magenta: '#c678dd',
  cyan: '#56b6c2',
  white: '#abb2bf',
  brightBlack: '#4b5263',
  brightRed: '#e06c75',
  brightGreen: '#98c379',
  brightYellow: '#e5c07b',
  brightBlue: '#61afef',
  brightMagenta: '#c678dd',
  brightCyan: '#56b6c2',
  brightWhite: '#d7dae0',
  extendedAnsi: EXTENDED_ANSI_ONE_DARK,
};

export function resolveTerminalTheme(theme: ResolvedTheme): ITheme {
  let resolvedTheme: ITheme;
  if (theme === 'light') {
    resolvedTheme = GITHUB_LIGHT_THEME;
  } else if (theme === 'darcula') {
    resolvedTheme = DARCULA_THEME;
  } else if (theme === 'one-dark-pro') {
    resolvedTheme = ONE_DARK_PRO_THEME;
  } else {
    resolvedTheme = GITHUB_DARK_THEME;
  }
  return applySurfaceBackground(resolvedTheme);
}
