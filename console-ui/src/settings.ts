import type { AppSettings } from './types';

const SETTINGS_KEY = 'octovalve.console.settings';

export const DEFAULT_SETTINGS: AppSettings = {
  notificationsEnabled: true,
  shortcuts: {
    jumpNextPending: 'n',
    approve: 'a',
    deny: 'd',
    fullScreen: 'r',
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
    return {
      ...DEFAULT_SETTINGS,
      ...parsed,
      shortcuts: {
        ...DEFAULT_SETTINGS.shortcuts,
        ...(parsed.shortcuts ?? {}),
      },
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
