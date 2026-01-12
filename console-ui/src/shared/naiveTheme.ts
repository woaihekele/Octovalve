import type { GlobalThemeOverrides } from 'naive-ui';
import type { ResolvedTheme } from './theme';

const DARCULA_PRIMARY = 'rgb(54, 88, 128)';
const DARCULA_PRIMARY_HOVER = 'rgb(62, 100, 145)';
const DARCULA_PRIMARY_PRESSED = 'rgb(43, 70, 103)';
const DARCULA_COLORS = {
  bg: '35 35 35',
  panel: '48 50 52',
  panelMuted: '42 44 46',
  border: '85 85 85',
  text: '200 200 200',
  textMuted: '160 160 160',
  accent: '104 151 187',
  accentSoft: '33 66 131',
  success: '152 195 121',
  warning: '204 120 50',
  danger: '224 108 117',
};

const formatRgb = (value: string) => {
  const trimmed = value.trim();
  if (trimmed.startsWith('rgb(')) {
    const inner = trimmed.slice(4, -1).trim();
    const normalized = inner.replace(/\s+/g, ', ').replace(/,\s*,/g, ', ');
    return `rgb(${normalized})`;
  }
  if (trimmed.includes(',')) {
    return `rgb(${trimmed})`;
  }
  return `rgb(${trimmed.replace(/\s+/g, ', ')})`;
};

export function resolveRgbVar(name: string, fallback: string) {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return formatRgb(fallback);
  }
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const value = raw || fallback;
  return formatRgb(value);
}

export function createNaiveThemeOverrides(resolvedTheme: ResolvedTheme): GlobalThemeOverrides {
  const isDarcula = resolvedTheme === 'darcula';
  const resolveColor = (name: string, fallback: string, darculaValue?: string) => {
    if (isDarcula && darculaValue) {
      return formatRgb(darculaValue);
    }
    return resolveRgbVar(name, fallback);
  };
  const overrides = {
    common: {
      primaryColor: isDarcula ? DARCULA_PRIMARY : resolveRgbVar('--color-accent', '99 102 241'),
      primaryColorHover: isDarcula ? DARCULA_PRIMARY_HOVER : resolveRgbVar('--color-accent', '99 102 241'),
      primaryColorPressed: isDarcula ? DARCULA_PRIMARY_PRESSED : resolveRgbVar('--color-accent-soft', '67 56 202'),
      primaryColorSuppl: isDarcula ? DARCULA_PRIMARY_PRESSED : resolveRgbVar('--color-accent-soft', '67 56 202'),
      successColor: resolveColor('--color-success', '52 211 153', DARCULA_COLORS.success),
      warningColor: resolveColor('--color-warning', '251 191 36', DARCULA_COLORS.warning),
      errorColor: resolveColor('--color-danger', '244 63 94', DARCULA_COLORS.danger),
      textColorBase: resolveColor('--color-text', '226 232 240', DARCULA_COLORS.text),
      textColor1: resolveColor('--color-text', '226 232 240', DARCULA_COLORS.text),
      textColor2: resolveColor('--color-text-muted', '100 116 139', DARCULA_COLORS.textMuted),
      textColor3: resolveColor('--color-text-muted', '100 116 139', DARCULA_COLORS.textMuted),
      placeholderColor: isDarcula ? 'rgb(170, 170, 170)' : resolveRgbVar('--color-text-muted', '100 116 139'),
      borderColor: resolveColor('--color-border', '51 65 85', DARCULA_COLORS.border),
      dividerColor: resolveColor('--color-border', '51 65 85', DARCULA_COLORS.border),
      bodyColor: resolveColor('--color-bg', '2 6 23', DARCULA_COLORS.bg),
      cardColor: resolveColor('--color-panel', '15 23 42', DARCULA_COLORS.panel),
      modalColor: resolveColor('--color-panel', '15 23 42', DARCULA_COLORS.panel),
      popoverColor: resolveColor('--color-panel', '15 23 42', DARCULA_COLORS.panel),
      inputColor: resolveColor('--color-panel-muted', '30 41 59', DARCULA_COLORS.panelMuted),
      actionColor: resolveColor('--color-panel-muted', '30 41 59', DARCULA_COLORS.panelMuted),
      actionColorHover: resolveColor('--color-panel-muted', '30 41 59', DARCULA_COLORS.panelMuted),
      actionColorPressed: resolveColor('--color-panel-muted', '30 41 59', DARCULA_COLORS.panelMuted),
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
  } as GlobalThemeOverrides;
  return overrides;
}
