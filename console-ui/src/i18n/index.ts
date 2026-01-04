import { createI18n } from 'vue-i18n';
import { messages } from './messages';
import type { AppLanguage } from '../shared/types';

export const i18n = createI18n({
  legacy: false,
  globalInjection: true,
  locale: 'en-US',
  fallbackLocale: 'en-US',
  messages,
});

export function setLocale(locale: AppLanguage) {
  i18n.global.locale.value = locale;
}
