import type { ThemeMode } from './types';

export type ResolvedTheme = 'dark' | 'light' | 'darcula';

export const THEME_OPTIONS = [
  { value: 'system', labelKey: 'theme.system' },
  { value: 'dark', labelKey: 'theme.dark' },
  { value: 'light', labelKey: 'theme.light' },
  { value: 'darcula', labelKey: 'theme.darcula' },
] as const;

export function isThemeMode(value: unknown): value is ThemeMode {
  return value === 'dark' || value === 'light' || value === 'system' || value === 'darcula';
}

export function normalizeThemeMode(value: unknown, fallback: ThemeMode): ThemeMode {
  return isThemeMode(value) ? value : fallback;
}

export function resolveTheme(mode: ThemeMode, prefersDark: boolean): ResolvedTheme {
  if (mode === 'system') {
    return prefersDark ? 'dark' : 'light';
  }
  if (mode === 'darcula') {
    return 'darcula';
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
