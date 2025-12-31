import { onBeforeUnmount, ref } from 'vue';
import type { ThemeMode } from '../types';

export function useThemeMode() {
  const resolvedTheme = ref<'dark' | 'light'>('dark');
  let stopSystemThemeListener: (() => void) | null = null;

  function updateResolvedTheme(resolved: 'dark' | 'light', mode: ThemeMode) {
    resolvedTheme.value = resolved;
    document.documentElement.dataset.theme = resolved;
    document.documentElement.dataset.themeMode = mode;
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
      const update = () => updateResolvedTheme(media.matches ? 'dark' : 'light', mode);
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
    updateResolvedTheme(mode, mode);
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
