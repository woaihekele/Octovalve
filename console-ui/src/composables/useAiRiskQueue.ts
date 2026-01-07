import { onBeforeUnmount, ref, type Ref } from 'vue';
import { aiRiskAssess } from '../services/api';
import { i18n } from '../i18n';
import type { AiRiskApiResponse, AiRiskEntry, AppSettings, RequestSnapshot, ServiceSnapshot } from '../shared/types';

const AI_RISK_CACHE_KEY = 'octovalve.console.ai_risk_cache';
const AI_RISK_CACHE_LIMIT = 2000;

type AiTask = {
  target: string;
  request: RequestSnapshot;
};

type AiRiskQueueOptions = {
  settings: Ref<AppSettings>;
  onError?: (context: string, err?: unknown) => void;
};

export function useAiRiskQueue({ settings, onError }: AiRiskQueueOptions) {
  const aiRiskMap = ref<Record<string, AiRiskEntry>>(loadAiRiskCache());
  const aiQueue = ref<AiTask[]>([]);
  const aiQueuedKeys = new Set<string>();
  const aiInFlightKeys = new Set<string>();
  const aiRunning = ref(0);
  let aiPumpScheduled = false;
  let aiPersistTimer: number | null = null;
  const t = i18n.global.t;

  function reportError(context: string, err?: unknown) {
    onError?.(context, err);
  }

  function loadAiRiskCache(): Record<string, AiRiskEntry> {
    if (typeof localStorage === 'undefined') {
      return {};
    }
    try {
      const raw = localStorage.getItem(AI_RISK_CACHE_KEY);
      if (!raw) {
        return {};
      }
      const parsed = JSON.parse(raw) as Record<string, AiRiskEntry>;
      if (!parsed || typeof parsed !== 'object') {
        return {};
      }
      const normalized: Record<string, AiRiskEntry> = {};
      for (const [key, value] of Object.entries(parsed)) {
        if (!value || typeof value !== 'object') {
          continue;
        }
        if (typeof value.updatedAt !== 'number') {
          continue;
        }
        normalized[key] = value;
      }
      return normalized;
    } catch (err) {
      reportError('ai risk cache load failed', err);
      return {};
    }
  }

  function scheduleAiRiskPersist() {
    if (typeof localStorage === 'undefined') {
      return;
    }
    if (aiPersistTimer !== null) {
      return;
    }
    aiPersistTimer = window.setTimeout(() => {
      aiPersistTimer = null;
      persistAiRiskCache();
    }, 500);
  }

  function persistAiRiskCache() {
    if (typeof localStorage === 'undefined') {
      return;
    }
    try {
      const entries = Object.entries(aiRiskMap.value);
      if (entries.length > AI_RISK_CACHE_LIMIT) {
        entries.sort(([, a], [, b]) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
        const trimmed = Object.fromEntries(entries.slice(0, AI_RISK_CACHE_LIMIT));
        aiRiskMap.value = trimmed;
      }
      localStorage.setItem(AI_RISK_CACHE_KEY, JSON.stringify(aiRiskMap.value));
    } catch (err) {
      reportError('ai risk cache persist failed', err);
    }
  }

  function setAiRisk(key: string, entry: AiRiskEntry) {
    aiRiskMap.value = {
      ...aiRiskMap.value,
      [key]: entry,
    };
    scheduleAiRiskPersist();
  }

  function updateAiRiskEntry(key: string, updates: Partial<AiRiskEntry>) {
    const existing = aiRiskMap.value[key];
    if (!existing) {
      return;
    }
    setAiRisk(key, { ...existing, ...updates });
  }

  function buildAiKey(targetName: string, requestId: string) {
    return `${targetName}:${requestId}`;
  }

  function formatAiPipeline(request: RequestSnapshot) {
    return request.pipeline.map((stage) => stage.argv.join(' ')).join(' | ');
  }

  function resolveAiModelConfig() {
    if (settings.value.ai.useChatModel) {
      return {
        baseUrl: settings.value.chat.openai.baseUrl,
        chatPath: settings.value.chat.openai.chatPath,
        model: settings.value.chat.openai.model,
        apiKey: settings.value.chat.openai.apiKey,
      };
    }
    return {
      baseUrl: settings.value.ai.baseUrl,
      chatPath: settings.value.ai.chatPath,
      model: settings.value.ai.model,
      apiKey: settings.value.ai.apiKey,
    };
  }

  function buildAiPrompt(template: string, targetName: string, request: RequestSnapshot) {
    const replacements: Record<string, string> = {
      target: targetName,
      client: request.client,
      peer: request.peer,
      intent: request.intent,
      mode: request.mode,
      raw_command: request.raw_command,
      pipeline: formatAiPipeline(request),
      cwd: request.cwd ?? '-',
      timeout_ms: request.timeout_ms?.toString() ?? '-',
      max_output_bytes: request.max_output_bytes?.toString() ?? '-',
    };
    return template.replace(/\{\{(\w+)\}\}/g, (match, key) => {
      if (key in replacements) {
        return replacements[key];
      }
      return match;
    });
  }

  function enqueueAiTask(targetName: string, request: RequestSnapshot) {
    if (!settings.value.ai.enabled) {
      return;
    }
    const key = buildAiKey(targetName, request.id);
    const existing = aiRiskMap.value[key];
    if (aiQueuedKeys.has(key) || aiInFlightKeys.has(key)) {
      return;
    }
    const { apiKey } = resolveAiModelConfig();
    if (!apiKey.trim()) {
      if (!existing || existing.status !== 'done') {
        setAiRisk(key, { status: 'error', error: t('aiRisk.error.noApiKey'), updatedAt: Date.now() });
      }
      return;
    }
    aiQueuedKeys.add(key);
    aiQueue.value = [...aiQueue.value, { target: targetName, request }];
    setAiRisk(key, { status: 'pending', updatedAt: Date.now() });
    scheduleAiQueue();
  }

  function scheduleAiQueue() {
    if (aiPumpScheduled) {
      return;
    }
    aiPumpScheduled = true;
    Promise.resolve().then(() => {
      aiPumpScheduled = false;
      processAiQueue();
    });
  }

  function resetAiQueue() {
    aiQueue.value = [];
    aiQueuedKeys.clear();
  }

  function processAiQueue() {
    if (!settings.value.ai.enabled) {
      resetAiQueue();
      return;
    }
    const maxConcurrency = Math.max(1, settings.value.ai.maxConcurrency);
    while (aiRunning.value < maxConcurrency && aiQueue.value.length > 0) {
      const task = aiQueue.value.shift();
      if (!task) {
        break;
      }
      const key = buildAiKey(task.target, task.request.id);
      aiQueuedKeys.delete(key);
      aiInFlightKeys.add(key);
      aiRunning.value += 1;
      void runAiTask(task)
        .catch((err) => {
          reportError('ai task failed', err);
          setAiRisk(key, { status: 'error', error: t('aiRisk.error.failed', { error: String(err) }), updatedAt: Date.now() });
        })
        .finally(() => {
          aiRunning.value -= 1;
          aiInFlightKeys.delete(key);
          scheduleAiQueue();
        });
    }
  }

  async function runAiTask(task: AiTask) {
    const key = buildAiKey(task.target, task.request.id);
    const now = Date.now();
    const { apiKey, baseUrl, chatPath, model } = resolveAiModelConfig();
    if (!apiKey.trim()) {
      setAiRisk(key, { status: 'error', error: t('aiRisk.error.noApiKey'), updatedAt: now });
      return;
    }
    try {
      const prompt = buildAiPrompt(settings.value.ai.prompt, task.target, task.request);
      const response = await aiRiskAssess({
        base_url: baseUrl,
        chat_path: chatPath,
        model,
        api_key: apiKey,
        prompt,
        timeout_ms: settings.value.ai.timeoutMs,
      });
      applyAiResult(key, response);
    } catch (err) {
      setAiRisk(key, { status: 'error', error: t('aiRisk.error.failed', { error: String(err) }), updatedAt: now });
      reportError('ai risk assess failed', err);
    }
  }

  function applyAiResult(key: string, response: AiRiskApiResponse) {
    const existing = aiRiskMap.value[key];
    const autoApproved = existing?.autoApproved === true;
    const autoApprovedAt = autoApproved ? existing?.autoApprovedAt : undefined;
    setAiRisk(key, {
      status: 'done',
      risk: response.risk,
      reason: response.reason ?? undefined,
      keyPoints: response.key_points ?? [],
      updatedAt: Date.now(),
      autoApproved,
      autoApprovedAt,
    });
  }

  function scheduleAiForSnapshot(targetName: string, snapshot: ServiceSnapshot) {
    if (!settings.value.ai.enabled) {
      return;
    }
    const pending = snapshot.queue ?? [];
    if (pending.length === 0) {
      return;
    }
    pending.forEach((item) => {
      const key = buildAiKey(targetName, item.id);
      const existing = aiRiskMap.value[key];
      if (
        !existing ||
        (existing.status === 'error' && existing.error?.includes('API Key') && resolveAiModelConfig().apiKey.trim())
      ) {
        enqueueAiTask(targetName, item);
      }
    });
  }

  onBeforeUnmount(() => {
    if (aiPersistTimer !== null) {
      window.clearTimeout(aiPersistTimer);
    }
  });

  return {
    aiRiskMap,
    enqueueAiTask,
    processAiQueue,
    scheduleAiForSnapshot,
    updateAiRiskEntry,
  };
}
