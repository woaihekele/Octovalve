import { onBeforeUnmount, ref } from 'vue';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { ThemeMode } from '../shared/types';
import { applyThemeToDocument, resolveTheme, type ResolvedTheme } from '../shared/theme';

export function useThemeMode() {
  const resolvedTheme = ref<ResolvedTheme>('dark');
  let stopSystemThemeListener: (() => void) | null = null;
  const isTauriRuntime = isTauri();

  function prefersDarkFromMedia() {
    const darkQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const lightQuery = window.matchMedia('(prefers-color-scheme: light)');
    if (lightQuery.matches && !darkQuery.matches) {
      return false;
    }
    if (darkQuery.matches && !lightQuery.matches) {
      return true;
    }
    return darkQuery.matches;
  }

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
      if (isTauriRuntime) {
        const appWindow = getCurrentWindow();
        let cancelled = false;
        let unlisten: (() => void) | null = null;
        const stop = () => {
          cancelled = true;
          if (unlisten) {
            unlisten();
          }
        };
        stopSystemThemeListener = stop;
        const applySystemTheme = (theme: 'light' | 'dark') => {
          if (cancelled) {
            return;
          }
          updateResolvedTheme(resolveTheme(mode, theme === 'dark'), mode);
        };
        const updateFromWindow = async () => {
          try {
            const theme = await appWindow.theme();
            if (theme === 'dark' || theme === 'light') {
              applySystemTheme(theme);
              return;
            }
          } catch {
            // Fall back to media query below.
          }
          applySystemTheme(prefersDarkFromMedia() ? 'dark' : 'light');
        };
        void updateFromWindow();
        appWindow.onThemeChanged(({ payload }) => {
          if (payload === 'dark' || payload === 'light') {
            applySystemTheme(payload);
          }
        }).then((fn) => {
          if (cancelled) {
            fn();
            return;
          }
          unlisten = fn;
        });
        return;
      }
      const media = window.matchMedia('(prefers-color-scheme: dark)');
      const update = () => updateResolvedTheme(resolveTheme(mode, prefersDarkFromMedia()), mode);
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
