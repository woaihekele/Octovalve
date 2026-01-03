<script setup lang="ts">
import { computed, provide } from 'vue';
import { NConfigProvider, NNotificationProvider, darkTheme } from 'naive-ui';
import ConsoleView from './ConsoleView.vue';
import { useThemeMode } from '../composables/useThemeMode';
import { APPLY_THEME_MODE, RESOLVED_THEME } from './appContext';

const { resolvedTheme, applyThemeMode } = useThemeMode();
provide(APPLY_THEME_MODE, applyThemeMode);
provide(RESOLVED_THEME, resolvedTheme);

function resolveRgbVar(name: string, fallback: string) {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return `rgb(${fallback})`;
  }
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const value = raw || fallback;
  if (value.startsWith('rgb')) {
    return value;
  }
  const normalized = value.replace(/\s+/g, ', ');
  return `rgb(${normalized})`;
}

const DARCULA_PRIMARY = 'rgb(54, 88, 128)';
const DARCULA_PRIMARY_HOVER = 'rgb(62, 100, 145)';
const DARCULA_PRIMARY_PRESSED = 'rgb(43, 70, 103)';

const naiveTheme = computed(() => (resolvedTheme.value === 'light' ? null : darkTheme));

const naiveThemeOverrides = computed(() => {
  void resolvedTheme.value;
  const isDarcula = resolvedTheme.value === 'darcula';
  return {
    common: {
      primaryColor: isDarcula ? DARCULA_PRIMARY : resolveRgbVar('--color-accent', '99 102 241'),
      primaryColorHover: isDarcula ? DARCULA_PRIMARY_HOVER : resolveRgbVar('--color-accent', '99 102 241'),
      primaryColorPressed: isDarcula ? DARCULA_PRIMARY_PRESSED : resolveRgbVar('--color-accent-soft', '67 56 202'),
      primaryColorSuppl: isDarcula ? DARCULA_PRIMARY_PRESSED : resolveRgbVar('--color-accent-soft', '67 56 202'),
      successColor: resolveRgbVar('--color-success', '52 211 153'),
      warningColor: resolveRgbVar('--color-warning', '251 191 36'),
      errorColor: resolveRgbVar('--color-danger', '244 63 94'),
      textColorBase: resolveRgbVar('--color-text', '226 232 240'),
      textColor1: resolveRgbVar('--color-text', '226 232 240'),
      textColor2: resolveRgbVar('--color-text-muted', '100 116 139'),
      textColor3: resolveRgbVar('--color-text-muted', '100 116 139'),
      placeholderColor: isDarcula ? 'rgb(170, 170, 170)' : resolveRgbVar('--color-text-muted', '100 116 139'),
      borderColor: resolveRgbVar('--color-border', '51 65 85'),
      dividerColor: resolveRgbVar('--color-border', '51 65 85'),
      bodyColor: resolveRgbVar('--color-bg', '2 6 23'),
      cardColor: resolveRgbVar('--color-panel', '15 23 42'),
      modalColor: resolveRgbVar('--color-panel', '15 23 42'),
      popoverColor: resolveRgbVar('--color-panel', '15 23 42'),
      inputColor: resolveRgbVar('--color-panel-muted', '30 41 59'),
      actionColor: resolveRgbVar('--color-panel-muted', '30 41 59'),
      actionColorHover: resolveRgbVar('--color-panel-muted', '30 41 59'),
      actionColorPressed: resolveRgbVar('--color-panel-muted', '30 41 59'),
    },
    Switch: {
      railColorActive: isDarcula ? DARCULA_PRIMARY : undefined,
    },
    Button: {
      colorPrimary: isDarcula ? DARCULA_PRIMARY : undefined,
      colorHoverPrimary: isDarcula ? DARCULA_PRIMARY_HOVER : undefined,
      colorPressedPrimary: isDarcula ? DARCULA_PRIMARY_PRESSED : undefined,
    },
    Tabs: {
      tabFontSizeSmall: '12px',
      tabHeightSmall: '24px',
      tabPaddingSmall: '0 10px',
      cardPaddingSmall: '0 4px',
      cardGapSmall: '4px',
    },
  };
});
</script>

<template>
  <n-config-provider :theme="naiveTheme" :theme-overrides="naiveThemeOverrides">
    <n-notification-provider>
      <ConsoleView />
    </n-notification-provider>
  </n-config-provider>
</template>
