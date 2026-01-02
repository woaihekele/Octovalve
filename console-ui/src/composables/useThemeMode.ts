import { onBeforeUnmount, ref } from 'vue';
import type { ThemeMode } from '../types';
import { applyThemeToDocument, resolveTheme, type ResolvedTheme } from '../theme';

export function useThemeMode() {
  const resolvedTheme = ref<ResolvedTheme>('dark');
  let stopSystemThemeListener: (() => void) | null = null;

  function updateResolvedTheme(resolved: ResolvedTheme, mode: ThemeMode) {
    resolvedTheme.value = resolved;
    applyThemeToDocument(resolved, mode);
  }

  function applyThemeMode(mode: ThemeMode) {
    if (typeof window === 'undefined' || typeof document === 'undefined') {
      return;
    }
    if (stopSystemThemeListener) {
      stopSystemThemeListener();
      stopSystemThemeListener = null;
    }
    if (mode === 'system') {
      const media = window.matchMedia('(prefers-color-scheme: dark)');
      const update = () => updateResolvedTheme(resolveTheme(mode, media.matches), mode);
      update();
      const handler = () => update();
      if (typeof media.addEventListener === 'function') {
        media.addEventListener('change', handler);
        stopSystemThemeListener = () => media.removeEventListener('change', handler);
      } else {
        media.addListener(handler);
        stopSystemThemeListener = () => media.removeListener(handler);
      }
      return;
    }
    updateResolvedTheme(resolveTheme(mode, false), mode);
  }

  onBeforeUnmount(() => {
    if (stopSystemThemeListener) {
      stopSystemThemeListener();
      stopSystemThemeListener = null;
    }
  });

  return {
    applyThemeMode,
    resolvedTheme,
  };
}
