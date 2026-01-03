import type { AiSettings, AppSettings, ChatProviderConfig } from '../shared/types';
import { normalizeShortcut } from '../shared/shortcuts';
import { normalizeThemeMode } from '../shared/theme';

const SETTINGS_KEY = 'octovalve.console.settings';
const DEFAULT_AI_PROMPT = [
  '你是命令风险评估助手。根据给定命令上下文评估风险，仅返回严格 JSON。',
  '要求：risk 为 low|medium|high；reason 一句话；key_points 为数组(可空)。',
  '输入:',
  'target={{target}}',
  'client={{client}}',
  'peer={{peer}}',
  'intent={{intent}}',
  'mode={{mode}}',
  'raw_command={{raw_command}}',
  'pipeline={{pipeline}}',
  'cwd={{cwd}}',
  'timeout_ms={{timeout_ms}}',
  'max_output_bytes={{max_output_bytes}}',
  '仅输出 JSON，不要输出解释。',
].join('\n');

const DEFAULT_AI_SETTINGS: AiSettings = {
  enabled: false,
  provider: 'openai',
  baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
  chatPath: '/chat/completions',
  model: 'glm-4.7',
  apiKey: '',
  prompt: DEFAULT_AI_PROMPT,
  timeoutMs: 10000,
  maxConcurrency: 2,
};

const DEFAULT_CHAT_SETTINGS: ChatProviderConfig = {
  provider: 'openai',
  openai: {
    baseUrl: 'https://api.openai.com/v1',
    apiKey: '',
    model: 'gpt-4o-mini',
    chatPath: '/chat/completions',
  },
  acp: {
    path: '',
  },
};

export const DEFAULT_SETTINGS: AppSettings = {
  notificationsEnabled: true,
  theme: 'system',
  ai: DEFAULT_AI_SETTINGS,
  chat: DEFAULT_CHAT_SETTINGS,
  shortcuts: {
    prevTarget: 'KeyW',
    nextTarget: 'KeyS',
    jumpNextPending: 'Meta+KeyN',
    approve: 'KeyA',
    deny: 'KeyD',
    fullScreen: 'KeyR',
    toggleList: 'Tab',
    openSettings: 'Meta+Comma',
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
    const parsedShortcuts: Partial<AppSettings['shortcuts']> = parsed.shortcuts ?? {};
    const parsedAi = (parsed.ai ?? {}) as Partial<AiSettings>;
    const normalizeWithFallback = (value: unknown, fallback: string) => {
      if (value === '') {
        return '';
      }
      if (typeof value !== 'string') {
        return fallback;
      }
      return normalizeShortcut(value) ?? fallback;
    };
    const normalizeAiProvider = (value: unknown) => (value === 'openai' ? 'openai' : DEFAULT_AI_SETTINGS.provider);
    const normalizeText = (value: unknown, fallback: string) => (typeof value === 'string' ? value : fallback);
    const normalizeNumber = (value: unknown, fallback: number, min: number, max: number) => {
      if (typeof value !== 'number' || Number.isNaN(value)) {
        return fallback;
      }
      if (value < min) {
        return min;
      }
      if (value > max) {
        return max;
      }
      return value;
    };
    const normalizedShortcuts = {
      prevTarget: normalizeWithFallback(parsedShortcuts.prevTarget, DEFAULT_SETTINGS.shortcuts.prevTarget),
      nextTarget: normalizeWithFallback(parsedShortcuts.nextTarget, DEFAULT_SETTINGS.shortcuts.nextTarget),
      jumpNextPending: normalizeWithFallback(
        parsedShortcuts.jumpNextPending,
        DEFAULT_SETTINGS.shortcuts.jumpNextPending
      ),
      approve: normalizeWithFallback(parsedShortcuts.approve, DEFAULT_SETTINGS.shortcuts.approve),
      deny: normalizeWithFallback(parsedShortcuts.deny, DEFAULT_SETTINGS.shortcuts.deny),
      fullScreen: normalizeWithFallback(parsedShortcuts.fullScreen, DEFAULT_SETTINGS.shortcuts.fullScreen),
      toggleList: normalizeWithFallback(parsedShortcuts.toggleList, DEFAULT_SETTINGS.shortcuts.toggleList),
      openSettings: normalizeWithFallback(parsedShortcuts.openSettings, DEFAULT_SETTINGS.shortcuts.openSettings),
    };
    const normalizedAi: AiSettings = {
      enabled: Boolean(parsedAi.enabled),
      provider: normalizeAiProvider(parsedAi.provider),
      baseUrl: normalizeText(parsedAi.baseUrl, DEFAULT_AI_SETTINGS.baseUrl),
      chatPath: normalizeText(parsedAi.chatPath, DEFAULT_AI_SETTINGS.chatPath),
      model: normalizeText(parsedAi.model, DEFAULT_AI_SETTINGS.model),
      apiKey: normalizeText(parsedAi.apiKey, DEFAULT_AI_SETTINGS.apiKey),
      prompt: normalizeText(parsedAi.prompt, DEFAULT_AI_SETTINGS.prompt),
      timeoutMs: normalizeNumber(parsedAi.timeoutMs, DEFAULT_AI_SETTINGS.timeoutMs, 1000, 60000),
      maxConcurrency: normalizeNumber(parsedAi.maxConcurrency, DEFAULT_AI_SETTINGS.maxConcurrency, 1, 10),
    };
    const parsedChat = (parsed.chat ?? {}) as Partial<ChatProviderConfig>;
    const normalizeChatProvider = (value: unknown): 'openai' | 'acp' => 
      (value === 'openai' || value === 'acp') ? value : DEFAULT_CHAT_SETTINGS.provider;
    const parsedOpenai = (parsedChat.openai ?? {}) as Partial<ChatProviderConfig['openai']>;
    const parsedAcp = (parsedChat.acp ?? {}) as Partial<ChatProviderConfig['acp']>;
    const normalizedChat: ChatProviderConfig = {
      provider: normalizeChatProvider(parsedChat.provider),
      openai: {
        baseUrl: normalizeText(parsedOpenai.baseUrl, DEFAULT_CHAT_SETTINGS.openai.baseUrl),
        apiKey: normalizeText(parsedOpenai.apiKey, DEFAULT_CHAT_SETTINGS.openai.apiKey),
        model: normalizeText(parsedOpenai.model, DEFAULT_CHAT_SETTINGS.openai.model),
        chatPath: normalizeText(parsedOpenai.chatPath, DEFAULT_CHAT_SETTINGS.openai.chatPath),
      },
      acp: {
        path: normalizeText(parsedAcp.path, DEFAULT_CHAT_SETTINGS.acp.path),
      },
    };
    return {
      ...DEFAULT_SETTINGS,
      ...parsed,
      theme: normalizeThemeMode(parsed.theme, DEFAULT_SETTINGS.theme),
      ai: normalizedAi,
      chat: normalizedChat,
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
