<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import {
  NAlert,
  NButton,
  NCheckbox,
  NCollapse,
  NCollapseItem,
  NDynamicInput,
  NDynamicTags,
  NIcon,
  NInput,
  NInputNumber,
  NSelect,
  NSpin,
  NSwitch,
} from 'naive-ui';
import type { SelectOption } from 'naive-ui';
import { useI18n } from 'vue-i18n';
import { AddOutline, TrashOutline } from '@vicons/ionicons5';
import MonacoEditor from '../MonacoEditor.vue';
import { parseBrokerConfigToml, parseProxyConfigToml } from '../../../services/api';
import { serializeBrokerConfigToml, serializeProxyConfigToml } from '../../../domain/config/toml';
import type { BrokerConfigEditor, ConfigFilePayload, ProxyConfigEditor, ProxyTargetConfig } from '../../../shared/types';
import type { ResolvedTheme } from '../../../shared/theme';

const props = defineProps<{
  configLoading: boolean;
  configBusy: boolean;
  logModalOpen: boolean;
  selectedProfile: string | null;
  profileOptions: SelectOption[];
  canDeleteProfile: boolean;
  highlight?: boolean;
  proxyConfig: ConfigFilePayload | null;
  brokerConfig: ConfigFilePayload | null;
  proxyConfigText: string;
  brokerConfigText: string;
  proxyDirty: boolean;
  brokerDirty: boolean;
  activeProfile: string | null;
  resolvedTheme: ResolvedTheme;
}>();

const emit = defineEmits<{
  (e: 'request-profile-change', value: string | null): void;
  (e: 'open-create-profile'): void;
  (e: 'open-delete-profile'): void;
  (e: 'request-refresh'): void;
  (e: 'close'): void;
  (e: 'save'): void;
  (e: 'apply'): void;
  (e: 'update:proxyConfigText', value: string): void;
  (e: 'update:brokerConfigText', value: string): void;
}>();

const { t } = useI18n();

type EditorMode = 'form' | 'toml';
type ArgRuleEntry = { command: string; pattern: string };
type CollapseName = string | number;
type CollapseNames = CollapseName | CollapseName[] | null;
type ProxyDefaultsForm = Omit<NonNullable<ProxyConfigEditor['defaults']>, 'ssh_args'> & {
  ssh_args: string[];
};
type ProxyTargetForm = Omit<ProxyTargetConfig, 'ssh_args'> & { ssh_args: string[] };
type ProxyConfigEditorForm = Omit<ProxyConfigEditor, 'defaults' | 'targets'> & {
  defaults: ProxyDefaultsForm;
  targets: ProxyTargetForm[];
};

const editorMode = ref<EditorMode>('form');
const TARGET_ADVANCED_NAME = 'target-advanced';
const PROXY_DEFAULTS_NAME = 'proxy-defaults';

const formBusy = ref(false);
const formError = ref<string | null>(null);
const proxyForm = ref<ProxyConfigEditorForm | null>(null);
const brokerForm = ref<BrokerConfigEditor | null>(null);
const brokerArgRules = ref<ArgRuleEntry[]>([]);
const selectedTargetIndex = ref<number | null>(null);
const targetAdvancedOpen = ref<boolean[]>([]);
const proxyDefaultsOpen = ref(false);

let suppressProxySync = false;
let suppressBrokerSync = false;
let lastProxyText: string | null = null;
let lastBrokerText: string | null = null;
let parseSeq = 0;

const proxyConfigModel = computed({
  get: () => props.proxyConfigText,
  set: (value: string) => emit('update:proxyConfigText', value),
});

const brokerConfigModel = computed({
  get: () => props.brokerConfigText,
  set: (value: string) => emit('update:brokerConfigText', value),
});

const textInputProps = {
  autocapitalize: 'off',
  autocorrect: 'off',
  spellcheck: false,
};

function normalizeStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : [];
}

function normalizeInputString(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

function collapseContains(value: CollapseNames, name: CollapseName): boolean {
  if (Array.isArray(value)) {
    return value.includes(name);
  }
  return value === name;
}

type ParsedSshDestination = { user: string; host: string };

function parseSshDestination(value: string): ParsedSshDestination {
  const trimmed = value.trim();
  if (!trimmed) {
    return { user: '', host: '' };
  }
  if (/\s/.test(trimmed)) {
    return { user: '', host: trimmed };
  }
  const at = trimmed.lastIndexOf('@');
  if (at > 0 && at < trimmed.length - 1) {
    return { user: trimmed.slice(0, at), host: trimmed.slice(at + 1) };
  }
  return { user: '', host: trimmed };
}

function formatSshDestination(user: string, host: string): string {
  const nextHost = host.trim();
  const nextUser = user.trim();
  if (!nextHost) {
    return '';
  }
  return nextUser ? `${nextUser}@${nextHost}` : nextHost;
}

function getTargetSshUser(target: ProxyTargetForm | null): string {
  if (!target) {
    return '';
  }
  return parseSshDestination(normalizeInputString(target.ssh)).user;
}

function getTargetSshHost(target: ProxyTargetForm | null): string {
  if (!target) {
    return '';
  }
  return parseSshDestination(normalizeInputString(target.ssh)).host;
}

function setTargetSshUser(target: ProxyTargetForm | null, user: string) {
  if (!target) {
    return;
  }
  const parsed = parseSshDestination(normalizeInputString(target.ssh));
  target.ssh = formatSshDestination(user, parsed.host);
}

function setTargetSshHost(target: ProxyTargetForm | null, host: string) {
  if (!target) {
    return;
  }
  const parsed = parseSshDestination(normalizeInputString(target.ssh));
  target.ssh = formatSshDestination(parsed.user, host);
}

function normalizeProxyForm(value: ProxyConfigEditor): ProxyConfigEditorForm {
  const defaults = (value.defaults ?? {}) as ProxyDefaultsForm;
  defaults.ssh_password = normalizeInputString(defaults.ssh_password);
  defaults.terminal_locale = normalizeInputString(defaults.terminal_locale);
  defaults.ssh_args = normalizeStringArray(defaults.ssh_args);
  const targets = (value.targets ?? []) as ProxyTargetForm[];
  for (const target of targets) {
    target.ssh = normalizeInputString(target.ssh);
    target.ssh_password = normalizeInputString(target.ssh_password);
    target.terminal_locale = normalizeInputString(target.terminal_locale);
    target.tty = Boolean(target.tty);
    target.ssh_args = normalizeStringArray(target.ssh_args);
  }
  return {
    broker_config_path: normalizeInputString(value.broker_config_path),
    default_target: value.default_target ?? null,
    defaults,
    targets,
  };
}

function normalizeBrokerForm(value: BrokerConfigEditor): BrokerConfigEditor {
  return {
    auto_approve_allowed: Boolean(value.auto_approve_allowed),
    whitelist: {
      allowed: normalizeStringArray(value.whitelist?.allowed),
      denied: normalizeStringArray(value.whitelist?.denied),
      arg_rules: value.whitelist?.arg_rules ?? {},
    },
    limits: {
      timeout_secs: Number(value.limits?.timeout_secs ?? 30),
      max_output_bytes: Number(value.limits?.max_output_bytes ?? 1024 * 1024),
    },
  };
}

function clampSelectedTarget(value: number | null, length: number): number | null {
  if (value === null) {
    return null;
  }
  if (!Number.isFinite(value) || value < 0 || value >= length) {
    return null;
  }
  return value;
}

function syncTargetUiState(length: number) {
  const next = targetAdvancedOpen.value.slice(0, length);
  while (next.length < length) {
    next.push(false);
  }
  targetAdvancedOpen.value = next;
  selectedTargetIndex.value = clampSelectedTarget(selectedTargetIndex.value, length);
}

function findDefaultTargetIndex(targets: ProxyTargetForm[]): number | null {
  const defaultName = proxyForm.value?.default_target?.trim();
  if (!defaultName) {
    return null;
  }
  const index = targets.findIndex((target) => target.name.trim() === defaultName);
  return index >= 0 ? index : null;
}

function ensureSelectedTarget(targets: ProxyTargetForm[]) {
  if (selectedTargetIndex.value !== null) {
    return;
  }
  const defaultIndex = findDefaultTargetIndex(targets);
  if (defaultIndex !== null) {
    selectedTargetIndex.value = defaultIndex;
  }
}

function selectTarget(index: number) {
  selectedTargetIndex.value = index;
}

function isDefaultTarget(name: string) {
  const trimmed = name.trim();
  const currentDefault = proxyForm.value?.default_target?.trim() ?? '';
  return Boolean(trimmed && currentDefault && trimmed === currentDefault);
}

async function loadInteractiveForms() {
  if (formBusy.value || props.configLoading) {
    return;
  }
  formBusy.value = true;
  formError.value = null;
  const current = ++parseSeq;
  try {
    const [proxy, broker] = await Promise.all([
      parseProxyConfigToml(props.proxyConfigText),
      parseBrokerConfigToml(props.brokerConfigText),
    ]);
    if (current !== parseSeq) {
      return;
    }
    suppressProxySync = true;
    suppressBrokerSync = true;
    proxyForm.value = normalizeProxyForm(proxy);
    brokerForm.value = normalizeBrokerForm(broker);
    syncTargetUiState(proxyForm.value.targets.length);
    ensureSelectedTarget(proxyForm.value.targets);
    brokerArgRules.value = Object.entries(brokerForm.value.whitelist.arg_rules ?? {}).map(
      ([command, pattern]) => ({ command, pattern })
    );
    await nextTick();
  } catch (err) {
    formError.value = String(err);
    editorMode.value = 'toml';
  } finally {
    suppressProxySync = false;
    suppressBrokerSync = false;
    formBusy.value = false;
  }
}

function syncProxyText() {
  const value = proxyForm.value;
  if (!value) {
    return;
  }
  const next = serializeProxyConfigToml(value);
  lastProxyText = next;
  emit('update:proxyConfigText', next);
}

function syncBrokerText() {
  const value = brokerForm.value;
  if (!value) {
    return;
  }
  const next = serializeBrokerConfigToml(value);
  lastBrokerText = next;
  emit('update:brokerConfigText', next);
}

function setEditorMode(value: string | null) {
  const next = (value === 'toml' ? 'toml' : 'form') as EditorMode;
  editorMode.value = next;
  formError.value = null;
  if (next === 'form') {
    void loadInteractiveForms();
  }
}

function addTarget() {
  if (!proxyForm.value) {
    return;
  }
  proxyForm.value.targets.push({
    name: '',
    desc: '',
    ssh: '',
    ssh_password: '',
    ssh_args: [],
    tty: false,
    terminal_locale: '',
  });
  targetAdvancedOpen.value.push(false);
  selectedTargetIndex.value = proxyForm.value.targets.length - 1;
}

function removeTarget(index: number) {
  if (!proxyForm.value) {
    return;
  }
  const removed = proxyForm.value.targets.splice(index, 1);
  if (targetAdvancedOpen.value.length > index) {
    targetAdvancedOpen.value.splice(index, 1);
  }
  if (selectedTargetIndex.value !== null) {
    if (selectedTargetIndex.value === index) {
      selectedTargetIndex.value = null;
    } else if (selectedTargetIndex.value > index) {
      selectedTargetIndex.value -= 1;
    }
  }
  const name = removed[0]?.name?.trim();
  if (name && proxyForm.value.default_target?.trim() === name) {
    proxyForm.value.default_target = null;
  }
  ensureSelectedTarget(proxyForm.value.targets);
}

function setDefaultTarget(name: string) {
  if (!proxyForm.value) {
    return;
  }
  const trimmed = name.trim();
  if (!trimmed) {
    return;
  }
  proxyForm.value.default_target = trimmed;
}

const targetAdvancedExpandedNames = computed(() => {
  if (selectedTargetIndex.value === null) {
    return [];
  }
  return targetAdvancedOpen.value[selectedTargetIndex.value] ? [TARGET_ADVANCED_NAME] : [];
});

const proxyDefaultsExpandedNames = computed(() => (proxyDefaultsOpen.value ? [PROXY_DEFAULTS_NAME] : []));

function updateTargetAdvancedExpanded(value: CollapseNames) {
  if (selectedTargetIndex.value === null) {
    return;
  }
  targetAdvancedOpen.value[selectedTargetIndex.value] = collapseContains(value, TARGET_ADVANCED_NAME);
}

function updateProxyDefaultsExpanded(value: CollapseNames) {
  proxyDefaultsOpen.value = collapseContains(value, PROXY_DEFAULTS_NAME);
}

const selectedTarget = computed(() => {
  const form = proxyForm.value;
  if (!form || selectedTargetIndex.value === null) {
    return null;
  }
  return form.targets[selectedTargetIndex.value] ?? null;
});

watch(
  () => props.configLoading,
  (loading) => {
    if (!loading && editorMode.value === 'form') {
      void loadInteractiveForms();
    }
  },
  { immediate: true }
);

watch(
  () => props.selectedProfile,
  () => {
    lastProxyText = null;
    lastBrokerText = null;
    selectedTargetIndex.value = null;
    targetAdvancedOpen.value = [];
    proxyDefaultsOpen.value = false;
    if (editorMode.value === 'form' && !props.configLoading) {
      void loadInteractiveForms();
    }
  }
);

watch(
  () => props.proxyConfigText,
  (value) => {
    if (editorMode.value !== 'form') {
      return;
    }
    if (lastProxyText !== null && value === lastProxyText) {
      return;
    }
    void loadInteractiveForms();
  }
);

watch(
  () => props.brokerConfigText,
  (value) => {
    if (editorMode.value !== 'form') {
      return;
    }
    if (lastBrokerText !== null && value === lastBrokerText) {
      return;
    }
    void loadInteractiveForms();
  }
);

watch(
  proxyForm,
  () => {
    if (editorMode.value !== 'form' || suppressProxySync) {
      return;
    }
    syncProxyText();
  },
  { deep: true }
);

watch(
  brokerForm,
  () => {
    if (editorMode.value !== 'form' || suppressBrokerSync) {
      return;
    }
    syncBrokerText();
  },
  { deep: true }
);

watch(
  brokerArgRules,
  (entries) => {
    if (editorMode.value !== 'form' || suppressBrokerSync) {
      return;
    }
    const map: Record<string, string> = {};
    for (const entry of entries) {
      const command = entry.command.trim();
      const pattern = entry.pattern.trim();
      if (!command || !pattern) {
        continue;
      }
      map[command] = pattern;
    }
    if (brokerForm.value) {
      brokerForm.value.whitelist.arg_rules = map;
    }
  },
  { deep: true }
);
</script>

<template>
  <div
    class="config-center-root flex h-full min-h-0 flex-1 flex-col overflow-hidden transition"
    :class="props.highlight ? 'ring-1 ring-accent/60 rounded-lg' : ''"
  >
    <div class="min-h-0 flex-1 overflow-auto pr-2">
      <div v-if="props.configLoading" class="flex items-center gap-2 text-sm text-foreground-muted">
        <NSpin size="small" />
        <span>{{ $t('settings.config.loading') }}</span>
      </div>
      <div v-else class="flex flex-col gap-4">
        <div class="flex flex-wrap items-center justify-between gap-3 text-sm">
          <div>
            <div class="font-medium">{{ $t('settings.profile.label') }}</div>
            <div class="text-xs text-foreground-muted">{{ $t('settings.profile.help') }}</div>
          </div>
          <div class="flex items-center gap-2">
            <NSelect
              :value="props.selectedProfile"
              :options="props.profileOptions"
              size="small"
              class="w-40"
              :placeholder="$t('settings.profile.placeholder')"
              :disabled="props.configBusy || props.logModalOpen || props.configLoading"
              @update:value="(v) => emit('request-profile-change', v as string | null)"
            />
            <NButton
              size="small"
              :disabled="props.configBusy || props.logModalOpen || props.configLoading"
              :aria-label="$t('common.create')"
              :title="$t('common.create')"
              @click="emit('open-create-profile')"
            >
              <template #icon>
                <NIcon :component="AddOutline" />
              </template>
            </NButton>
            <NButton
              size="small"
              quaternary
              :disabled="props.configBusy || props.logModalOpen || props.configLoading || !props.canDeleteProfile"
              :aria-label="$t('common.delete')"
              :title="$t('common.delete')"
              @click="emit('open-delete-profile')"
            >
              <template #icon>
                <NIcon :component="TrashOutline" />
              </template>
            </NButton>
          </div>
        </div>

        <div class="flex flex-wrap items-center justify-between gap-3 text-sm">
          <div>
            <div class="font-medium">{{ $t('settings.config.editor.title') }}</div>
            <div class="text-xs text-foreground-muted">{{ $t('settings.config.editor.help') }}</div>
          </div>
          <NCheckbox
            :checked="editorMode === 'toml'"
            size="small"
            :disabled="props.configBusy || props.logModalOpen || props.configLoading"
            @update:checked="(v) => setEditorMode(v ? 'toml' : 'form')"
          >
            {{ $t('settings.config.editor.toml') }}
          </NCheckbox>
        </div>

        <NAlert
          v-if="formError"
          type="error"
          :title="$t('settings.config.editor.parseFailed')"
          class="text-sm"
        >
          <div class="break-all">{{ formError }}</div>
        </NAlert>

        <div
          :class="editorMode === 'form'
            ? 'grid grid-cols-1 gap-4 pb-2'
            : 'grid grid-cols-1 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)] gap-4 pb-2'"
        >
          <div class="rounded-lg border border-border/50 bg-panel-muted/30">
            <div class="flex items-start justify-between gap-3 border-b border-border/40 px-3 py-2 text-sm">
              <div class="min-w-0">
                <div class="font-medium">{{ $t('settings.config.proxyTitle') }}</div>
                <div class="text-xs text-foreground-muted break-all">{{ props.proxyConfig?.path }}</div>
              </div>
              <span
                v-if="props.proxyConfig && !props.proxyConfig.exists"
                class="shrink-0 text-xs text-warning"
              >
                {{ $t('settings.config.missing') }}
              </span>
            </div>
            <div class="p-3">
              <div v-if="editorMode === 'toml'">
                <MonacoEditor v-model="proxyConfigModel" language="toml" height="52vh" :theme="props.resolvedTheme" />
              </div>
              <div v-else>
                <div v-if="!proxyForm || formBusy" class="flex items-center gap-2 text-xs text-foreground-muted">
                  <NSpin size="small" />
                  <span>{{ $t('settings.config.editor.parsing') }}</span>
                </div>
                <div v-else class="space-y-4">
                  <div class="rounded-lg border border-border/40 p-3">
                    <div class="flex items-center justify-between gap-2">
                      <div class="text-sm font-medium">{{ $t('settings.config.proxy.targetsTitle') }}</div>
                      <NButton
                        size="small"
                        type="primary"
                        :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                        @click="addTarget"
                      >
                        <template #icon>
                          <NIcon :component="AddOutline" />
                        </template>
                        {{ $t('settings.config.proxy.addTarget') }}
                      </NButton>
                    </div>
                    <div class="mt-3 grid grid-cols-1 gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
                      <div>
                        <div v-if="proxyForm.targets.length === 0" class="text-xs text-foreground-muted">
                          {{ $t('settings.config.proxy.noTargets') }}
                        </div>
                        <div v-else class="space-y-2">
                          <div
                            v-for="(target, index) in proxyForm.targets"
                            :key="index"
                            class="rounded-lg border px-3 py-2 transition"
                            :class="selectedTargetIndex === index ? 'border-accent/40 bg-panel-muted/40' : 'border-border/40 bg-panel-muted/30 hover:border-border/60'"
                          >
                            <div class="flex items-start justify-between gap-2">
                              <button
                                type="button"
                                class="flex min-w-0 flex-1 flex-col gap-1 text-left"
                                :aria-pressed="selectedTargetIndex === index"
                                @click="selectTarget(index)"
                              >
                                <div class="flex min-w-0 items-center gap-2">
                                  <div class="min-w-0 truncate text-sm font-medium">
                                    {{ target.name?.trim() ? target.name : $t('settings.config.proxy.unnamedTarget') }}
                                  </div>
                                  <span
                                    v-if="isDefaultTarget(target.name)"
                                    class="shrink-0 rounded bg-accent/15 px-2 py-0.5 text-[11px] text-accent"
                                  >
                                    {{ $t('console.default') }}
                                  </span>
                                </div>
                                <div
                                  class="min-w-0 truncate text-xs text-foreground-muted"
                                  :title="[getTargetSshHost(target) || '-', getTargetSshUser(target) || '-'].join(' · ')"
                                >
                                  {{ getTargetSshHost(target) || '-' }}
                                  <span class="mx-1">·</span>
                                  {{ getTargetSshUser(target) || '-' }}
                                </div>
                              </button>
                              <div class="flex items-center gap-2">
                                <NButton
                                  v-if="!isDefaultTarget(target.name)"
                                  size="tiny"
                                  type="primary"
                                  :disabled="props.configBusy || props.logModalOpen || props.configLoading || !target.name.trim()"
                                  @click.stop="setDefaultTarget(target.name)"
                                >
                                  {{ $t('settings.config.proxy.setDefault') }}
                                </NButton>
                                <NButton
                                  size="tiny"
                                  quaternary
                                  type="error"
                                  :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                                  @click.stop="removeTarget(index)"
                                >
                                  <template #icon>
                                    <NIcon :component="TrashOutline" />
                                  </template>
                                </NButton>
                              </div>
                            </div>
                          </div>
                        </div>
                      </div>
                      <div class="rounded-lg border border-border/40 p-3">
                        <div v-if="selectedTarget && selectedTargetIndex !== null" class="space-y-3">
                          <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
                            <div class="flex flex-col gap-1">
                              <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.name') }}</div>
                              <NInput
                                v-model:value="selectedTarget.name"
                                size="small"
                                :placeholder="$t('settings.config.fields.namePlaceholder')"
                                :input-props="textInputProps"
                                :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                              />
                            </div>
                            <div class="flex flex-col gap-1">
                              <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.sshHost') }}</div>
                              <NInput
                                :value="getTargetSshHost(selectedTarget)"
                                size="small"
                                :placeholder="$t('settings.config.fields.sshHostPlaceholder')"
                                :input-props="textInputProps"
                                :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                                @update:value="(v) => setTargetSshHost(selectedTarget, v)"
                              />
                            </div>
                            <div class="flex flex-col gap-1">
                              <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.sshUser') }}</div>
                              <NInput
                                :value="getTargetSshUser(selectedTarget)"
                                size="small"
                                :placeholder="$t('settings.config.fields.sshUserPlaceholder')"
                                :input-props="textInputProps"
                                :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                                @update:value="(v) => setTargetSshUser(selectedTarget, v)"
                              />
                            </div>
                            <div class="flex flex-col gap-1">
                              <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.sshPassword') }}</div>
                              <NInput
                                v-model:value="selectedTarget.ssh_password"
                                type="password"
                                show-password-on="click"
                                size="small"
                                :placeholder="$t('settings.config.fields.passwordPlaceholder')"
                                :input-props="textInputProps"
                                :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                              />
                            </div>
                            <div class="flex flex-col gap-1 md:col-span-2">
                              <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.desc') }}</div>
                              <NInput
                                v-model:value="selectedTarget.desc"
                                type="textarea"
                                :autosize="{ minRows: 2, maxRows: 4 }"
                                size="small"
                                :placeholder="$t('settings.config.fields.descPlaceholder')"
                                :input-props="textInputProps"
                                :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                              />
                            </div>
                          </div>

                          <div class="rounded-lg border border-border/40 p-3">
                            <NCollapse
                              :expanded-names="targetAdvancedExpandedNames"
                              arrow-placement="right"
                              display-directive="show"
                              :trigger-areas="['main', 'arrow']"
                              @update:expanded-names="updateTargetAdvancedExpanded"
                            >
                              <NCollapseItem :name="TARGET_ADVANCED_NAME">
                                <template #header>
                                  <div class="text-xs text-foreground-muted">
                                    {{ $t('settings.config.proxy.advanced') }}
                                  </div>
                                </template>
                                <div class="pt-3 grid grid-cols-1 gap-3 md:grid-cols-2">
                                  <div class="flex flex-col gap-1 md:col-span-2">
                                    <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.sshArgs') }}</div>
                                    <NDynamicTags
                                      v-model:value="selectedTarget.ssh_args"
                                      size="small"
                                      :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                                    />
                                  </div>
                                  <div class="flex flex-col gap-1 md:col-span-2">
                                    <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.terminalLocale') }}</div>
                                    <NInput
                                      v-model:value="selectedTarget.terminal_locale"
                                      size="small"
                                      :placeholder="$t('settings.config.fields.terminalLocalePlaceholder')"
                                      :input-props="textInputProps"
                                      :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                                    />
                                  </div>
                                  <div class="flex items-center justify-between gap-3 md:col-span-2">
                                    <span class="text-xs text-foreground-muted">{{ $t('settings.config.fields.tty') }}</span>
                                    <NSwitch
                                      v-model:value="selectedTarget.tty"
                                      size="small"
                                      :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                                    />
                                  </div>
                                </div>
                              </NCollapseItem>
                            </NCollapse>
                          </div>
                        </div>
                        <div v-else class="text-xs text-foreground-muted">
                          {{
                            proxyForm.targets.length === 0
                              ? $t('settings.config.proxy.noTargets')
                              : $t('settings.config.proxy.selectTargetHint')
                          }}
                        </div>
                      </div>
                    </div>
                  </div>

                  <div class="rounded-lg border border-border/40 p-3">
                    <NCollapse
                      :expanded-names="proxyDefaultsExpandedNames"
                      arrow-placement="right"
                      display-directive="show"
                      :trigger-areas="['main', 'arrow']"
                      @update:expanded-names="updateProxyDefaultsExpanded"
                    >
                      <NCollapseItem :name="PROXY_DEFAULTS_NAME">
                        <template #header>
                          <div class="text-sm font-medium">{{ $t('settings.config.proxy.advanced') }}</div>
                        </template>
                        <div class="pt-3 space-y-4">
                      <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
                        <div class="flex flex-col gap-1">
                          <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.brokerConfigPath') }}</div>
                          <NInput
                            v-model:value="proxyForm.broker_config_path"
                            size="small"
                            :placeholder="$t('settings.config.fields.brokerConfigPathPlaceholder')"
                            :input-props="textInputProps"
                            :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                          />
                        </div>
                      </div>
                      <div class="text-sm font-medium">{{ $t('settings.config.proxy.defaultsTitle') }}</div>
                      <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
                        <div class="flex flex-col gap-1">
                          <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.timeoutMs') }}</div>
                          <NInputNumber
                            v-model:value="proxyForm.defaults.timeout_ms"
                            size="small"
                            :min="0"
                            :precision="0"
                            :show-button="false"
                            :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                          />
                        </div>
                        <div class="flex flex-col gap-1">
                          <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.maxOutputBytes') }}</div>
                          <NInputNumber
                            v-model:value="proxyForm.defaults.max_output_bytes"
                            size="small"
                            :min="0"
                            :precision="0"
                            :show-button="false"
                            :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                          />
                        </div>
                        <div class="flex flex-col gap-1 md:col-span-2">
                          <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.sshArgs') }}</div>
                          <NDynamicTags
                            v-model:value="proxyForm.defaults.ssh_args"
                            size="small"
                            :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                          />
                        </div>
                        <div class="flex flex-col gap-1">
                          <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.sshPassword') }}</div>
                          <NInput
                            v-model:value="proxyForm.defaults.ssh_password"
                            type="password"
                            show-password-on="click"
                            size="small"
                            :placeholder="$t('settings.config.fields.passwordPlaceholder')"
                            :input-props="textInputProps"
                            :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                          />
                        </div>
                        <div class="flex flex-col gap-1">
                          <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.terminalLocale') }}</div>
                          <NInput
                            v-model:value="proxyForm.defaults.terminal_locale"
                            size="small"
                            :placeholder="$t('settings.config.fields.terminalLocalePlaceholder')"
                            :input-props="textInputProps"
                            :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                          />
                        </div>
                      </div>
                        </div>
                      </NCollapseItem>
                    </NCollapse>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div class="rounded-lg border border-border/50 bg-panel-muted/30">
            <div class="flex items-start justify-between gap-3 border-b border-border/40 px-3 py-2 text-sm">
              <div class="min-w-0">
                <div class="font-medium">{{ $t('settings.config.brokerTitle') }}</div>
                <div class="text-xs text-foreground-muted break-all">{{ props.brokerConfig?.path }}</div>
              </div>
            </div>
            <div class="p-3">
              <div v-if="editorMode === 'toml'">
                <MonacoEditor v-model="brokerConfigModel" language="toml" height="52vh" :theme="props.resolvedTheme" />
              </div>
              <div v-else>
                <div v-if="!brokerForm || formBusy" class="flex items-center gap-2 text-xs text-foreground-muted">
                  <NSpin size="small" />
                  <span>{{ $t('settings.config.editor.parsing') }}</span>
                </div>
                <div v-else class="space-y-4">
                  <div class="flex items-center justify-between gap-3">
                    <span class="text-xs text-foreground-muted">{{ $t('settings.config.fields.autoApproveAllowed') }}</span>
                    <NSwitch
                      v-model:value="brokerForm.auto_approve_allowed"
                      size="small"
                      :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                    />
                  </div>

                  <div class="space-y-4">
                    <div class="rounded-lg border border-border/40 p-3">
                      <div class="text-sm font-medium">{{ $t('settings.config.broker.limitsTitle') }}</div>
                      <div class="mt-3 grid grid-cols-1 gap-3 md:grid-cols-2">
                        <div class="flex flex-col gap-1">
                          <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.timeoutSecs') }}</div>
                          <NInputNumber
                            v-model:value="brokerForm.limits.timeout_secs"
                            size="small"
                            :min="1"
                            :precision="0"
                            :show-button="false"
                            :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                          />
                        </div>
                        <div class="flex flex-col gap-1">
                          <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.maxOutputBytes') }}</div>
                          <NInputNumber
                            v-model:value="brokerForm.limits.max_output_bytes"
                            size="small"
                            :min="0"
                            :precision="0"
                            :show-button="false"
                            :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                          />
                        </div>
                      </div>
                    </div>

                    <div class="space-y-3">
                      <div class="text-sm font-medium">{{ $t('settings.config.broker.whitelistTitle') }}</div>
                      <div class="flex flex-col gap-1">
                        <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.allowed') }}</div>
                        <NDynamicTags
                          v-model:value="brokerForm.whitelist.allowed"
                          size="small"
                          :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                        />
                      </div>
                      <div class="flex flex-col gap-1">
                        <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.denied') }}</div>
                        <NDynamicTags
                          v-model:value="brokerForm.whitelist.denied"
                          size="small"
                          :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                        />
                      </div>
                      <div class="flex flex-col gap-2">
                        <div class="text-xs text-foreground-muted">{{ $t('settings.config.fields.argRules') }}</div>
                        <NDynamicInput
                          v-model:value="brokerArgRules"
                          size="small"
                          :on-create="() => ({ command: '', pattern: '' })"
                          :disabled="props.configBusy || props.logModalOpen || props.configLoading"
                        >
                          <template #="{ value }">
                            <div class="grid grid-cols-1 gap-2 md:grid-cols-2 w-full">
                              <NInput
                                v-model:value="value.command"
                                size="small"
                                :placeholder="$t('settings.config.fields.argRuleCommandPlaceholder')"
                                :input-props="textInputProps"
                              />
                              <NInput
                                v-model:value="value.pattern"
                                size="small"
                                :placeholder="$t('settings.config.fields.argRulePatternPlaceholder')"
                                :input-props="textInputProps"
                              />
                            </div>
                          </template>
                        </NDynamicInput>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

  </div>
</template>

<style scoped>
.config-center-root :deep(.n-input:not(.n-input--disabled) .n-input__input input),
.config-center-root :deep(.n-input:not(.n-input--disabled) .n-input__textarea textarea),
.config-center-root :deep(.n-input-number:not(.n-input-number--disabled) .n-input__input input),
.config-center-root :deep(.n-base-selection:not(.n-base-selection--disabled) .n-base-selection-label) {
  color: rgb(var(--color-text));
}

.config-center-root :deep(.n-input:not(.n-input--disabled) .n-input__input input::placeholder),
.config-center-root :deep(.n-input:not(.n-input--disabled) .n-input__textarea textarea::placeholder),
.config-center-root :deep(.n-input-number:not(.n-input-number--disabled) .n-input__input input::placeholder),
.config-center-root :deep(.n-base-selection:not(.n-base-selection--disabled) .n-base-selection-placeholder) {
  color: rgb(var(--color-text-muted));
}
</style>
