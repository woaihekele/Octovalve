import type { InjectionKey, Ref } from 'vue';
import type { ResolvedTheme } from './theme';
import type { ThemeMode } from './types';

export const APPLY_THEME_MODE: InjectionKey<(mode: ThemeMode) => void> = Symbol('applyThemeMode');
export const RESOLVED_THEME: InjectionKey<Ref<ResolvedTheme>> = Symbol('resolvedTheme');
