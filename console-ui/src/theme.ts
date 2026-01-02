import type { ThemeMode } from './types';

export type ResolvedTheme = 'dark' | 'light';

export const THEME_OPTIONS = [
  { value: 'system', label: '系统' },
  { value: 'dark', label: '深色' },
  { value: 'light', label: '浅色' },
] as const;

export function isThemeMode(value: unknown): value is ThemeMode {
  return value === 'dark' || value === 'light' || value === 'system';
}

export function normalizeThemeMode(value: unknown, fallback: ThemeMode): ThemeMode {
  return isThemeMode(value) ? value : fallback;
}

export function resolveTheme(mode: ThemeMode, prefersDark: boolean): ResolvedTheme {
  if (mode === 'system') {
    return prefersDark ? 'dark' : 'light';
  }
  return mode;
}

export function applyThemeToDocument(resolved: ResolvedTheme, mode: ThemeMode) {
  if (typeof document === 'undefined') {
    return;
  }
  document.documentElement.dataset.theme = resolved;
  document.documentElement.dataset.themeMode = mode;
}
