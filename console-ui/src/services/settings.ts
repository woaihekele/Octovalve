import type { AiSettings, AppLanguage, AppSettings, ChatProviderConfig } from '../shared/types';
import { normalizeShortcut } from '../shared/shortcuts';
import { normalizeThemeMode } from '../shared/theme';

const SETTINGS_KEY = 'octovalve.console.settings';
const DEFAULT_LANGUAGE: AppLanguage = 'en-US';
const DEFAULT_AI_PROMPTS: Record<AppLanguage, string> = {
  'zh-CN': [
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
  ].join('\n'),
  'en-US': [
    'You are a command risk assessment assistant. Assess risk based on the given command context and return strict JSON only.',
    'Requirements: risk is low|medium|high; reason is one sentence; key_points is an array (can be empty).',
    'Input:',
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
    'Output JSON only. No explanations.',
  ].join('\n'),
};

export function getDefaultAiPrompt(language: AppLanguage): string {
  return DEFAULT_AI_PROMPTS[language] ?? DEFAULT_AI_PROMPTS[DEFAULT_LANGUAGE];
}

const BASE_AI_SETTINGS: Omit<AiSettings, 'prompt'> = {
  enabled: false,
  autoApproveLowRisk: false,
  useChatModel: false,
  provider: 'openai',
  baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
  chatPath: '/chat/completions',
  model: 'glm-4.7',
  apiKey: '',
  timeoutMs: 10000,
  maxConcurrency: 2,
};

function buildDefaultAiSettings(language: AppLanguage): AiSettings {
  return {
    ...BASE_AI_SETTINGS,
    prompt: getDefaultAiPrompt(language),
  };
}

function detectSystemLanguage(): AppLanguage {
  if (typeof navigator === 'undefined') {
    return DEFAULT_LANGUAGE;
  }
  const language =
    (Array.isArray(navigator.languages) && navigator.languages[0]) ||
    navigator.language ||
    '';
  if (language.toLowerCase().startsWith('zh')) {
    return 'zh-CN';
  }
  return 'en-US';
}

function buildDefaultSettings(language: AppLanguage): AppSettings {
  return {
    ...DEFAULT_SETTINGS,
    language,
    ai: buildDefaultAiSettings(language),
  };
}

const DEFAULT_AI_SETTINGS: AiSettings = buildDefaultAiSettings(DEFAULT_LANGUAGE);
const DEFAULT_UI_SCALE = 1;
const DEFAULT_TERMINAL_SCALE = 1;

const DEFAULT_CHAT_SETTINGS: ChatProviderConfig = {
  provider: 'openai',
  sendOnEnter: false,
  openai: {
    baseUrl: 'https://api.openai.com/v1',
    apiKey: '',
    model: 'gpt-4o-mini',
    chatPath: '/chat/completions',
  },
  acp: {
    path: '',
    args: '',
    approvalPolicy: 'on-request',
    sandboxMode: 'workspace-write',
  },
};

export const DEFAULT_SETTINGS: AppSettings = {
  notificationsEnabled: true,
  theme: 'system',
  language: DEFAULT_LANGUAGE,
  uiScale: DEFAULT_UI_SCALE,
  terminalScale: DEFAULT_TERMINAL_SCALE,
  ai: DEFAULT_AI_SETTINGS,
  chat: DEFAULT_CHAT_SETTINGS,
  shortcuts: {
    prevTarget: 'KeyW',
    nextTarget: 'KeyS',
    jumpNextPending: 'Meta+KeyN',
    approve: 'KeyA',
    deny: 'KeyD',
    fullScreen: 'KeyR',
    openSettings: 'Meta+Comma',
  },
};

export function loadSettings(): AppSettings {
  if (typeof localStorage === 'undefined') {
    return buildDefaultSettings(detectSystemLanguage());
  }
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (!raw) {
      return buildDefaultSettings(detectSystemLanguage());
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
    const normalizeBool = (value: unknown, fallback: boolean) => (typeof value === 'boolean' ? value : fallback);
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
      openSettings: normalizeWithFallback(parsedShortcuts.openSettings, DEFAULT_SETTINGS.shortcuts.openSettings),
    };
    const normalizedUiScale = normalizeNumber(parsed.uiScale, DEFAULT_SETTINGS.uiScale, 0.8, 1.5);
    const normalizedTerminalScale = normalizeNumber(
      parsed.terminalScale,
      DEFAULT_SETTINGS.terminalScale,
      0.8,
      1.5
    );
    const normalizeLanguage = (value: unknown, fallback: AppLanguage): AppLanguage =>
      value === 'en-US' || value === 'zh-CN' ? value : fallback;
    const normalizedLanguage = normalizeLanguage(parsed.language, detectSystemLanguage());
    const defaultAiSettings = buildDefaultAiSettings(normalizedLanguage);
    const normalizedAi: AiSettings = {
      enabled: Boolean(parsedAi.enabled),
      autoApproveLowRisk: normalizeBool(parsedAi.autoApproveLowRisk, defaultAiSettings.autoApproveLowRisk),
      useChatModel: normalizeBool(parsedAi.useChatModel, defaultAiSettings.useChatModel),
      provider: normalizeAiProvider(parsedAi.provider),
      baseUrl: normalizeText(parsedAi.baseUrl, defaultAiSettings.baseUrl),
      chatPath: normalizeText(parsedAi.chatPath, defaultAiSettings.chatPath),
      model: normalizeText(parsedAi.model, defaultAiSettings.model),
      apiKey: normalizeText(parsedAi.apiKey, defaultAiSettings.apiKey),
      prompt: normalizeText(parsedAi.prompt, defaultAiSettings.prompt),
      timeoutMs: normalizeNumber(parsedAi.timeoutMs, defaultAiSettings.timeoutMs, 1000, 60000),
      maxConcurrency: normalizeNumber(parsedAi.maxConcurrency, defaultAiSettings.maxConcurrency, 1, 10),
    };
    const parsedChat = (parsed.chat ?? {}) as Partial<ChatProviderConfig>;
    const normalizeChatProvider = (value: unknown): 'openai' | 'acp' =>
      (value === 'openai' || value === 'acp') ? value : DEFAULT_CHAT_SETTINGS.provider;
    const normalizeAcpApprovalPolicy = (
      value: unknown,
      fallback: ChatProviderConfig['acp']['approvalPolicy']
    ) =>
      value === 'auto' ||
      value === 'unless-trusted' ||
      value === 'on-failure' ||
      value === 'on-request' ||
      value === 'never'
        ? value
        : fallback;
    const normalizeAcpSandboxMode = (
      value: unknown,
      fallback: ChatProviderConfig['acp']['sandboxMode']
    ) =>
      value === 'auto' ||
      value === 'read-only' ||
      value === 'workspace-write' ||
      value === 'danger-full-access'
        ? value
        : fallback;
    const parsedOpenai = (parsedChat.openai ?? {}) as Partial<ChatProviderConfig['openai']>;
    const parsedAcp = (parsedChat.acp ?? {}) as Partial<ChatProviderConfig['acp']>;
    const normalizedChat: ChatProviderConfig = {
      provider: normalizeChatProvider(parsedChat.provider),
      sendOnEnter: normalizeBool(parsedChat.sendOnEnter, DEFAULT_CHAT_SETTINGS.sendOnEnter),
      openai: {
        baseUrl: normalizeText(parsedOpenai.baseUrl, DEFAULT_CHAT_SETTINGS.openai.baseUrl),
        apiKey: normalizeText(parsedOpenai.apiKey, DEFAULT_CHAT_SETTINGS.openai.apiKey),
        model: normalizeText(parsedOpenai.model, DEFAULT_CHAT_SETTINGS.openai.model),
        chatPath: normalizeText(parsedOpenai.chatPath, DEFAULT_CHAT_SETTINGS.openai.chatPath),
      },
      acp: {
        path: normalizeText(parsedAcp.path, DEFAULT_CHAT_SETTINGS.acp.path),
        args: normalizeText(parsedAcp.args, DEFAULT_CHAT_SETTINGS.acp.args),
        approvalPolicy: normalizeAcpApprovalPolicy(
          parsedAcp.approvalPolicy,
          DEFAULT_CHAT_SETTINGS.acp.approvalPolicy
        ),
        sandboxMode: normalizeAcpSandboxMode(
          parsedAcp.sandboxMode,
          DEFAULT_CHAT_SETTINGS.acp.sandboxMode
        ),
      },
    };
    return {
      ...DEFAULT_SETTINGS,
      ...parsed,
      theme: normalizeThemeMode(parsed.theme, DEFAULT_SETTINGS.theme),
      language: normalizedLanguage,
      uiScale: normalizedUiScale,
      terminalScale: normalizedTerminalScale,
      ai: normalizedAi,
      chat: normalizedChat,
      shortcuts: normalizedShortcuts,
    };
  } catch {
    return buildDefaultSettings(detectSystemLanguage());
  }
}

export function saveSettings(settings: AppSettings) {
  if (typeof localStorage === 'undefined') {
    return;
  }
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
}
