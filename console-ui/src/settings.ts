import type { AppSettings } from './types';
import { normalizeShortcut } from './shortcuts';

const SETTINGS_KEY = 'octovalve.console.settings';

export const DEFAULT_SETTINGS: AppSettings = {
  notificationsEnabled: true,
  shortcuts: {
    jumpNextPending: 'Meta+KeyN',
    approve: 'KeyA',
    deny: 'KeyD',
    fullScreen: 'KeyR',
    toggleList: 'Tab',
  },
};

export function loadSettings(): AppSettings {
  if (typeof localStorage === 'undefined') {
    return DEFAULT_SETTINGS;
  }
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (!raw) {
      return DEFAULT_SETTINGS;
    }
    const parsed = JSON.parse(raw) as Partial<AppSettings>;
    const parsedShortcuts = parsed.shortcuts ?? {};
    const normalizedShortcuts = {
      jumpNextPending:
        normalizeShortcut(parsedShortcuts.jumpNextPending ?? '') ?? DEFAULT_SETTINGS.shortcuts.jumpNextPending,
      approve: normalizeShortcut(parsedShortcuts.approve ?? '') ?? DEFAULT_SETTINGS.shortcuts.approve,
      deny: normalizeShortcut(parsedShortcuts.deny ?? '') ?? DEFAULT_SETTINGS.shortcuts.deny,
      fullScreen: normalizeShortcut(parsedShortcuts.fullScreen ?? '') ?? DEFAULT_SETTINGS.shortcuts.fullScreen,
      toggleList: normalizeShortcut(parsedShortcuts.toggleList ?? '') ?? DEFAULT_SETTINGS.shortcuts.toggleList,
    };
    return {
      ...DEFAULT_SETTINGS,
      ...parsed,
      shortcuts: normalizedShortcuts,
    };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

export function saveSettings(settings: AppSettings) {
  if (typeof localStorage === 'undefined') {
    return;
  }
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
}
