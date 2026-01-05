<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { NButton, NPopover, NTag } from 'naive-ui';
import { useI18n } from 'vue-i18n';
import { formatShortcut, matchesShortcut } from '../../shared/shortcuts';
import type {
  AiRiskEntry,
  AppSettings,
  RequestSnapshot,
  RunningSnapshot,
  ResultSnapshot,
  ServiceSnapshot,
  TargetInfo,
} from '../../shared/types';
const props = defineProps<{
  target: TargetInfo;
  snapshot: ServiceSnapshot | null;
  settings: AppSettings;
  pendingJumpToken: number;
  terminalOpen: boolean;
  chatOpen: boolean;
  aiRiskMap: Record<string, AiRiskEntry>;
  aiEnabled: boolean;
}>();

const emit = defineEmits<{
  (e: 'approve', id: string): void;
  (e: 'deny', id: string): void;
  (e: 'cancel', id: string): void;
  (e: 'open-terminal'): void;
  (e: 'close-terminal'): void;
  (e: 'toggle-chat'): void;
  (e: 'refresh-risk', payload: { target: string; id: string }): void;
}>();

const { t, locale } = useI18n();

const selectedId = ref<string | null>(null);
const isFullScreen = ref(false);
const splitContainerRef = ref<HTMLDivElement | null>(null);
const terminalContainerRef = ref<HTMLDivElement | null>(null);
const terminalHeight = ref<number | null>(null);
const containerHeight = ref(0);
const isResizing = ref(false);
const lastPendingCount = ref(0);
const terminalHeightStorageKey = 'console-ui.target-terminal.height';
const minTerminalHeight = 240;
const minContentHeight = 240;
let resizeObserver: ResizeObserver | null = null;
let resizeStartY = 0;
let resizeStartHeight = 0;

type SnapshotItem = RequestSnapshot | RunningSnapshot | ResultSnapshot;

const pendingList = computed(() => props.snapshot?.queue ?? []);
const runningList = computed(() => props.snapshot?.running ?? []);
const historyList = computed(() => props.snapshot?.history ?? []);
const combinedList = computed(() => [
  ...pendingList.value,
  ...runningList.value,
  ...historyList.value,
]);
const terminalStyle = computed(() => {
  if (!props.terminalOpen) {
    return {};
  }
  const fallback = containerHeight.value > 0 ? containerHeight.value / 2 : minTerminalHeight;
  const height = terminalHeight.value ?? fallback;
  return { height: `${clampTerminalHeight(height)}px` };
});
const selectedIndex = computed(() => {
  if (!selectedId.value) {
    return combinedList.value.length > 0 ? 0 : -1;
  }
  return combinedList.value.findIndex((item) => item.id === selectedId.value);
});
const selectedItem = computed<SnapshotItem | null>(() => {
  const index = selectedIndex.value;
  if (index < 0) {
    return null;
  }
  return combinedList.value[index] ?? null;
});
const isPendingSelected = computed(() => (selectedItem.value ? isPendingItem(selectedItem.value) : false));
const isRunningSelected = computed(() => (selectedItem.value ? isRunningItem(selectedItem.value) : false));
const hostDisplay = computed(() => {
  const hostname = props.target.hostname?.trim();
  const ip = props.target.ip?.trim();
  if (hostname && ip && hostname !== ip) {
    return `${hostname} / ${ip}`;
  }
  return hostname || ip || 'unknown';
});

watch(
  () => props.target.name,
  () => {
    selectByIndex(0);
  }
);

watch(
  () => props.pendingJumpToken,
  () => {
    const firstPending = pendingList.value[0];
    if (firstPending) {
      selectedId.value = firstPending.id;
      return;
    }
    selectByIndex(0);
  }
);

watch(
  () => pendingList.value.length,
  (length) => {
    if (length === 0) {
      lastPendingCount.value = 0;
      return;
    }
    if (lastPendingCount.value === 0) {
      selectedId.value = pendingList.value[0]?.id ?? null;
    }
    lastPendingCount.value = length;
  },
  { immediate: true }
);

watch(
  () => combinedList.value,
  (list) => {
    if (list.length === 0) {
      selectedId.value = null;
      return;
    }
    if (!selectedId.value || !list.some((item) => item.id === selectedId.value)) {
      selectedId.value = list[0].id;
    }
  }
);

function formatTime(value: number) {
  return new Date(value).toLocaleString(locale.value);
}

function itemTimestamp(item: SnapshotItem) {
  if (isPendingItem(item)) {
    return item.received_at_ms;
  }
  if (isRunningItem(item)) {
    return item.started_at_ms;
  }
  return item.finished_at_ms;
}

function isResultItem(item: SnapshotItem): item is ResultSnapshot {
  return 'finished_at_ms' in item;
}

function isRunningItem(item: SnapshotItem): item is RunningSnapshot {
  return 'started_at_ms' in item;
}

function isPendingItem(item: SnapshotItem): item is RequestSnapshot {
  return !isResultItem(item) && !isRunningItem(item);
}

function formatSummary(result: ResultSnapshot) {
  if (result.status === 'completed') {
    return t('target.summary.completed', { exit: result.exit_code ?? 'n/a' });
  }
  if (result.status === 'denied') {
    return t('target.status.denied');
  }
  if (result.status === 'error') {
    return t('target.status.error');
  }
  if (result.status === 'cancelled') {
    return t('target.status.cancelled');
  }
  return result.status;
}

function resultStatusLabel(status: ResultSnapshot['status']) {
  if (status === 'completed') {
    return t('target.status.completed');
  }
  if (status === 'denied') {
    return t('target.status.denied');
  }
  if (status === 'error') {
    return t('target.status.error');
  }
  if (status === 'cancelled') {
    return t('target.status.cancelled');
  }
  return status;
}

function aiKey(id: string) {
  return `${props.target.name}:${id}`;
}

function getAiEntry(id: string) {
  return props.aiRiskMap[aiKey(id)];
}

function aiEntryForItem(item: SnapshotItem) {
  return getAiEntry(item.id);
}

function aiLabel(entry?: AiRiskEntry) {
  if (!entry) {
    return t('target.ai.status.unknown');
  }
  if (entry.status === 'pending') {
    return t('target.ai.status.pending');
  }
  if (entry.status === 'error') {
    if (entry.error?.includes('API Key')) {
      return t('target.ai.status.unconfigured');
    }
    return t('target.ai.status.failed');
  }
  if (entry.risk === 'low') {
    return t('target.ai.risk.low');
  }
  if (entry.risk === 'medium') {
    return t('target.ai.risk.medium');
  }
  if (entry.risk === 'high') {
    return t('target.ai.risk.high');
  }
  return t('common.unknown');
}

function aiTagType(entry?: AiRiskEntry) {
  if (!entry) {
    return 'default';
  }
  if (entry.status === 'pending') {
    return 'info';
  }
  if (entry.status === 'error') {
    return entry.error?.includes('API Key') ? 'warning' : 'error';
  }
  if (entry.risk === 'low') {
    return 'success';
  }
  if (entry.risk === 'medium') {
    return 'warning';
  }
  if (entry.risk === 'high') {
    return 'error';
  }
  return 'default';
}

function shouldShowAiTag(item: SnapshotItem) {
  if (isPendingItem(item)) {
    return props.aiEnabled;
  }
  return Boolean(getAiEntry(item.id));
}

function aiAutoApproved(entry?: AiRiskEntry) {
  return entry?.autoApproved === true;
}

function aiAutoApprovedLabel(entry?: AiRiskEntry) {
  return aiAutoApproved(entry) ? t('common.yes') : t('common.no');
}

function aiAutoApprovedTime(entry?: AiRiskEntry) {
  return entry?.autoApprovedAt ? formatTime(entry.autoApprovedAt) : '-';
}

const selectedAiEntry = computed(() => (selectedItem.value ? getAiEntry(selectedItem.value.id) : undefined));
const showSelectedAiTag = computed(() => (selectedItem.value ? shouldShowAiTag(selectedItem.value) : false));

function refreshRisk(id: string) {
  emit('refresh-risk', { target: props.target.name, id });
}

function selectByIndex(index: number) {
  const item = combinedList.value[index];
  selectedId.value = item ? item.id : null;
}

function buildOutput(result: ResultSnapshot) {
  const stdout = result.stdout?.trim();
  const stderr = result.stderr?.trim();
  if (stdout && stderr) {
    return `${stdout}\n\n---- stderr ----\n${stderr}`;
  }
  if (stdout) {
    return stdout;
  }
  if (stderr) {
    return stderr;
  }
  return '';
}

function handleKeyDown(event: KeyboardEvent) {
  if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) {
    return;
  }
  if (props.terminalOpen && terminalContainerRef.value) {
    const target = event.target instanceof Node ? event.target : null;
    if (target && terminalContainerRef.value.contains(target)) {
      return;
    }
    const activeElement = document.activeElement;
    if (activeElement && terminalContainerRef.value.contains(activeElement)) {
      return;
    }
  } else if (props.terminalOpen) {
    return;
  }

  const key = event.key;

  if (key === 'j' || key === 'ArrowDown') {
    event.preventDefault();
    if (combinedList.value.length === 0) {
      return;
    }
    const nextIndex = Math.min(Math.max(selectedIndex.value, 0) + 1, combinedList.value.length - 1);
    selectByIndex(nextIndex);
    return;
  }

  if (key === 'k' || key === 'ArrowUp') {
    event.preventDefault();
    if (combinedList.value.length === 0) {
      return;
    }
    const nextIndex = Math.max(Math.max(selectedIndex.value, 0) - 1, 0);
    selectByIndex(nextIndex);
    return;
  }

  if (matchesShortcut(event, props.settings.shortcuts.fullScreen)) {
    event.preventDefault();
    isFullScreen.value = !isFullScreen.value;
    return;
  }

  if (key === 'Escape' && isFullScreen.value) {
    isFullScreen.value = false;
    return;
  }

  if (selectedItem.value && isPendingItem(selectedItem.value)) {
    if (matchesShortcut(event, props.settings.shortcuts.approve)) {
      emit('approve', selectedItem.value.id);
    } else if (matchesShortcut(event, props.settings.shortcuts.deny)) {
      emit('deny', selectedItem.value.id);
    }
  }
}

onMounted(() => window.addEventListener('keydown', handleKeyDown));
onBeforeUnmount(() => window.removeEventListener('keydown', handleKeyDown));

function handleTerminalToggle() {
  if (props.terminalOpen) {
    emit('close-terminal');
    return;
  }
  emit('open-terminal');
}

function readStoredTerminalHeight() {
  if (typeof window === 'undefined') {
    return null;
  }
  const raw = window.localStorage.getItem(terminalHeightStorageKey);
  if (!raw) {
    return null;
  }
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return null;
  }
  return parsed;
}

function persistTerminalHeight() {
  if (typeof window === 'undefined' || terminalHeight.value === null) {
    return;
  }
  window.localStorage.setItem(terminalHeightStorageKey, String(terminalHeight.value));
}

function clampTerminalHeight(value: number) {
  if (containerHeight.value <= 0) {
    return value;
  }
  const min = Math.min(minTerminalHeight, containerHeight.value);
  const max = Math.max(min, containerHeight.value - minContentHeight);
  return Math.min(max, Math.max(min, value));
}

function updateContainerHeight() {
  if (!splitContainerRef.value) {
    return;
  }
  const nextHeight = Math.round(splitContainerRef.value.getBoundingClientRect().height);
  if (!nextHeight || nextHeight === containerHeight.value) {
    return;
  }
  containerHeight.value = nextHeight;
  if (terminalHeight.value !== null) {
    terminalHeight.value = clampTerminalHeight(terminalHeight.value);
  }
}

function ensureTerminalHeight() {
  if (terminalHeight.value !== null || containerHeight.value <= 0) {
    return;
  }
  terminalHeight.value = clampTerminalHeight(containerHeight.value / 2);
}

function handleResizeMove(event: MouseEvent) {
  const dy = resizeStartY - event.clientY;
  terminalHeight.value = clampTerminalHeight(resizeStartHeight + dy);
}

function stopResize() {
  if (!isResizing.value) {
    return;
  }
  isResizing.value = false;
  window.removeEventListener('mousemove', handleResizeMove);
  window.removeEventListener('mouseup', stopResize);
  persistTerminalHeight();
}

function startResize(event: MouseEvent) {
  if (!props.terminalOpen) {
    return;
  }
  isResizing.value = true;
  resizeStartY = event.clientY;
  resizeStartHeight = terminalHeight.value ?? clampTerminalHeight(containerHeight.value / 2);
  window.addEventListener('mousemove', handleResizeMove);
  window.addEventListener('mouseup', stopResize);
}

terminalHeight.value = readStoredTerminalHeight();

watch(
  () => props.terminalOpen,
  (open) => {
    if (!open) {
      return;
    }
    updateContainerHeight();
    ensureTerminalHeight();
  }
);

onMounted(() => {
  updateContainerHeight();
  ensureTerminalHeight();
  if (typeof ResizeObserver !== 'undefined' && splitContainerRef.value) {
    resizeObserver = new ResizeObserver(() => {
      updateContainerHeight();
    });
    resizeObserver.observe(splitContainerRef.value);
  }
});

onBeforeUnmount(() => {
  if (resizeObserver) {
    resizeObserver.disconnect();
    resizeObserver = null;
  }
  window.removeEventListener('mousemove', handleResizeMove);
  window.removeEventListener('mouseup', stopResize);
});
</script>

<template>
  <div class="flex flex-col h-full bg-surface" :class="isFullScreen ? 'fixed inset-0 z-40' : 'relative'">
    <div
      v-if="!isFullScreen"
      class="h-16 border-b border-border flex items-center justify-between px-6 bg-panel/50"
      data-tauri-drag-region
    >
      <div class="flex items-center gap-4 min-w-0">
        <div class="min-w-0">
          <div class="flex items-baseline gap-3 min-w-0">
            <h2 class="text-xl font-semibold text-foreground">{{ props.target.name }}</h2>
            <span class="text-sm text-foreground-muted truncate max-w-[360px]" :title="hostDisplay">
              {{ hostDisplay }}
            </span>
          </div>
          <div class="text-sm text-foreground-muted">{{ props.target.desc }}</div>
          <div
            v-if="props.target.last_error"
            class="text-xs text-danger mt-1 max-w-[520px] truncate"
            :title="props.target.last_error"
          >
            {{ props.target.last_error }}
          </div>
        </div>
      </div>
      <div class="flex items-center gap-2">
        <button
          class="p-2 rounded border transition-colors"
          :class="
            props.target.terminal_available
              ? props.terminalOpen
                ? 'bg-panel/70 text-foreground border-accent/60 ring-1 ring-accent/40'
                : 'bg-panel/60 text-foreground border-border hover:border-accent/40'
              : 'bg-panel/30 text-foreground-muted border-border/60 cursor-not-allowed'
          "
          :disabled="!props.target.terminal_available"
          @click="handleTerminalToggle"
        :aria-label="props.terminalOpen ? $t('target.terminal.close') : $t('target.terminal.open')"
        :title="props.terminalOpen ? $t('target.terminal.close') : $t('target.terminal.open')"
        >
          <svg
            class="h-4 w-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <rect x="3" y="4" width="18" height="16" rx="2"></rect>
            <polyline points="8 9 11 12 8 15"></polyline>
            <line x1="13" y1="15" x2="17" y2="15"></line>
          </svg>
        </button>

        <button
          class="p-2 rounded border transition-colors"
          :class="
            props.chatOpen
              ? 'bg-panel/70 text-foreground border-accent/60 ring-1 ring-accent/40'
              : 'bg-panel/60 text-foreground border-border hover:border-accent/40'
          "
          @click="emit('toggle-chat')"
        :aria-label="props.chatOpen ? $t('chat.toggle.close') : $t('chat.toggle.open')"
        :title="props.chatOpen ? $t('chat.toggle.close') : $t('chat.toggle.open')"
        >
          <svg
            class="h-4 w-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <line x1="12" y1="3" x2="12" y2="6" />
            <circle cx="12" cy="3" r="1" />
            <rect x="4" y="6" width="16" height="12" rx="3" />
            <circle cx="9" cy="12" r="1.2" fill="currentColor" stroke="none" />
            <circle cx="15" cy="12" r="1.2" fill="currentColor" stroke="none" />
            <line x1="9" y1="15" x2="15" y2="15" />
          </svg>
        </button>
      </div>
    </div>

    <div ref="splitContainerRef" class="flex-1 flex flex-col overflow-hidden min-h-0">
      <div class="flex-1 min-h-0 flex overflow-hidden">
        <div v-if="!isFullScreen" class="w-1/3 min-w-[320px] border-r border-border flex flex-col bg-panel/20 min-h-0">
          <div class="flex items-center justify-between border-b border-border px-4 py-3">
            <div class="text-sm font-medium text-foreground">{{ $t('target.list.title') }}</div>
            <div class="flex items-center gap-2 text-xs text-foreground-muted">
              <span class="bg-panel-muted px-1.5 py-0.5 rounded-full text-foreground">
                {{ $t('target.list.pending') }} {{ pendingList.length }}
              </span>
              <span class="bg-panel-muted px-1.5 py-0.5 rounded-full text-foreground">
                {{ $t('target.list.running') }} {{ runningList.length }}
              </span>
              <span class="bg-panel-muted px-1.5 py-0.5 rounded-full text-foreground">
                {{ $t('target.list.history') }} {{ historyList.length }}
              </span>
            </div>
          </div>

          <div class="flex-1 overflow-y-auto min-h-0">
            <div v-if="combinedList.length === 0" class="p-8 text-center text-foreground-muted text-sm">
              {{ $t('target.list.empty') }}
            </div>
            <div
              v-for="(item, index) in combinedList"
              :key="item.id"
              class="p-4 border-b border-border cursor-pointer transition-colors"
              :class="item.id === selectedId ? 'bg-accent/20 border-l-4 border-l-accent' : 'hover:bg-panel-muted/30 border-l-4 border-l-transparent'"
              @click="selectedId = item.id"
            >
              <div class="flex justify-between items-start mb-1 gap-2">
                <span class="min-w-0 flex-1 font-mono text-sm line-clamp-1" :class="item.id === selectedId ? 'text-accent' : 'text-foreground'">
                  {{ item.raw_command }}
                </span>
                <div class="flex items-center gap-2 shrink-0">
                  <span
                    v-if="isPendingItem(item)"
                    class="text-xs px-2 py-0.5 rounded bg-accent/20 text-accent whitespace-nowrap"
                  >
                    {{ $t('target.status.pending') }}
                  </span>
                  <span
                    v-else-if="isRunningItem(item)"
                    class="text-xs px-2 py-0.5 rounded bg-panel-muted text-foreground whitespace-nowrap"
                  >
                    {{ $t('target.status.running') }}
                  </span>
                  <span
                    v-else
                    class="text-xs px-2 py-0.5 rounded whitespace-nowrap"
                    :class="(item as ResultSnapshot).status === 'completed'
                      ? 'bg-success/20 text-success'
                      : (item as ResultSnapshot).status === 'denied'
                        ? 'bg-danger/20 text-danger'
                        : (item as ResultSnapshot).status === 'cancelled'
                          ? 'bg-panel-muted text-foreground-muted'
                          : 'bg-warning/20 text-warning'"
                  >
                    {{ resultStatusLabel((item as ResultSnapshot).status) }}
                  </span>
                  <n-popover v-if="shouldShowAiTag(item)" trigger="hover" placement="left" :delay="120">
                    <template #trigger>
                      <div class="flex items-center gap-1">
                        <n-tag size="small" :type="aiTagType(aiEntryForItem(item))" :bordered="false">
                          {{ aiLabel(aiEntryForItem(item)) }}
                        </n-tag>
                        <n-tag v-if="aiAutoApproved(aiEntryForItem(item))" size="small" type="warning" :bordered="false">
                          {{ $t('target.ai.autoTag') }}
                        </n-tag>
                      </div>
                    </template>
                    <div class="space-y-2 text-xs max-w-[260px]">
                      <div class="font-medium text-foreground">{{ $t('target.ai.title') }}</div>
                      <div v-if="aiEntryForItem(item)?.status === 'pending'" class="text-foreground-muted">{{ $t('target.ai.checking') }}</div>
                      <div v-else-if="aiEntryForItem(item)?.status === 'error'" class="text-danger">
                        {{ aiEntryForItem(item)?.error || $t('target.ai.failed') }}
                      </div>
                      <template v-else>
                        <div>
                          <span class="text-foreground-muted">{{ $t('target.ai.level') }}</span>
                          <span class="text-foreground">{{ aiLabel(aiEntryForItem(item)) }}</span>
                        </div>
                        <div v-if="aiEntryForItem(item)?.reason">
                          <span class="text-foreground-muted">{{ $t('target.ai.reason') }}</span>
                          <span class="text-foreground">{{ aiEntryForItem(item)?.reason }}</span>
                        </div>
                        <div v-if="aiEntryForItem(item)?.keyPoints?.length">
                          <div class="text-foreground-muted mb-1">{{ $t('target.ai.keyPoints') }}</div>
                          <div class="space-y-1">
                            <div v-for="(point, pIndex) in aiEntryForItem(item)?.keyPoints" :key="pIndex">
                              - {{ point }}
                            </div>
                          </div>
                        </div>
                        <div v-if="aiEntryForItem(item)?.status === 'done'">
                          <span class="text-foreground-muted">{{ $t('target.ai.autoApproved') }}</span>
                          <span class="text-foreground">{{ aiAutoApprovedLabel(aiEntryForItem(item)) }}</span>
                        </div>
                        <div v-if="aiEntryForItem(item)?.autoApprovedAt" class="text-foreground-muted">
                          {{ $t('target.ai.autoApprovedAt', { time: aiAutoApprovedTime(aiEntryForItem(item)) }) }}
                        </div>
                      </template>
                      <div class="text-foreground-muted">
                        {{ $t('target.ai.updatedAt', { time: aiEntryForItem(item)?.updatedAt ? formatTime(aiEntryForItem(item)!.updatedAt) : '-' }) }}
                      </div>
                      <div v-if="props.aiEnabled && isPendingItem(item)" class="flex justify-end">
                        <n-button size="tiny" @click.stop="refreshRisk(item.id)">{{ $t('common.refresh') }}</n-button>
                      </div>
                    </div>
                  </n-popover>
                </div>
              </div>
              <div class="flex justify-between items-center text-xs text-foreground-muted">
                <span>
                  {{ formatTime(itemTimestamp(item)) }}
                </span>
                <span v-if="isPendingItem(item)" class="truncate">{{ (item as RequestSnapshot).intent }}</span>
              </div>
            </div>
          </div>
        </div>

        <div class="flex-1 flex flex-col">
          <template v-if="selectedItem">
            <div class="border-b border-border bg-panel/30 p-6 flex justify-between gap-6">
              <div class="flex-1">
                <h3 class="text-xs font-semibold text-foreground-muted uppercase tracking-wider mb-2">{{ $t('target.detail.command') }}</h3>
                <code
                  class="block text-base text-accent font-mono bg-panel px-4 py-3 rounded-lg border border-border max-h-40 overflow-y-auto whitespace-pre-wrap break-words"
                >
                  {{ selectedItem.raw_command }}
                </code>

                <div v-if="showSelectedAiTag" class="mt-3 flex items-center gap-2">
                  <n-popover trigger="hover" placement="left" :delay="120">
                    <template #trigger>
                      <div class="flex items-center gap-1">
                        <n-tag size="small" :type="aiTagType(selectedAiEntry)" :bordered="false">
                          {{ aiLabel(selectedAiEntry) }}
                        </n-tag>
                        <n-tag v-if="aiAutoApproved(selectedAiEntry)" size="small" type="warning" :bordered="false">
                          {{ $t('target.ai.autoTag') }}
                        </n-tag>
                      </div>
                    </template>
                    <div class="space-y-2 text-xs max-w-[260px]">
                      <div class="font-medium text-foreground">{{ $t('target.ai.title') }}</div>
                      <div v-if="selectedAiEntry?.status === 'pending'" class="text-foreground-muted">{{ $t('target.ai.checking') }}</div>
                      <div v-else-if="selectedAiEntry?.status === 'error'" class="text-danger">
                        {{ selectedAiEntry?.error || $t('target.ai.failed') }}
                      </div>
                      <template v-else>
                        <div>
                          <span class="text-foreground-muted">{{ $t('target.ai.level') }}</span>
                          <span class="text-foreground">{{ aiLabel(selectedAiEntry) }}</span>
                        </div>
                        <div v-if="selectedAiEntry?.reason">
                          <span class="text-foreground-muted">{{ $t('target.ai.reason') }}</span>
                          <span class="text-foreground">{{ selectedAiEntry?.reason }}</span>
                        </div>
                        <div v-if="selectedAiEntry?.keyPoints?.length">
                          <div class="text-foreground-muted mb-1">{{ $t('target.ai.keyPoints') }}</div>
                          <div class="space-y-1">
                            <div v-for="(point, pIndex) in selectedAiEntry?.keyPoints" :key="pIndex">
                              - {{ point }}
                            </div>
                          </div>
                        </div>
                        <div v-if="selectedAiEntry?.status === 'done'">
                          <span class="text-foreground-muted">{{ $t('target.ai.autoApproved') }}</span>
                          <span class="text-foreground">{{ aiAutoApprovedLabel(selectedAiEntry) }}</span>
                        </div>
                        <div v-if="selectedAiEntry?.autoApprovedAt" class="text-foreground-muted">
                          {{ $t('target.ai.autoApprovedAt', { time: aiAutoApprovedTime(selectedAiEntry) }) }}
                        </div>
                      </template>
                      <div class="text-foreground-muted">
                        {{ $t('target.ai.updatedAt', { time: selectedAiEntry?.updatedAt ? formatTime(selectedAiEntry!.updatedAt) : '-' }) }}
                      </div>
                      <div v-if="props.aiEnabled && isPendingSelected" class="flex justify-end">
                        <n-button size="tiny" @click.stop="refreshRisk(selectedItem.id)">{{ $t('common.refresh') }}</n-button>
                      </div>
                    </div>
                  </n-popover>
                </div>

                <div class="mt-4 grid grid-cols-2 gap-4 text-xs text-foreground-muted">
                  <div>
                    <div class="text-foreground-muted">{{ $t('target.detail.intent') }}</div>
                    <div class="text-foreground">{{ selectedItem.intent }}</div>
                  </div>
                  <div>
                    <div class="text-foreground-muted">{{ $t('target.detail.mode') }}</div>
                    <div class="text-foreground">{{ selectedItem.mode }}</div>
                  </div>
                  <div>
                    <div class="text-foreground-muted">{{ $t('target.detail.cwd') }}</div>
                    <div class="text-foreground">{{ selectedItem.cwd || '-' }}</div>
                  </div>
                  <div>
                    <div class="text-foreground-muted">{{ $t('target.detail.peer') }}</div>
                    <div class="text-foreground">{{ selectedItem.peer }}</div>
                  </div>
                  <template v-if="isPendingSelected">
                    <div>
                      <div class="text-foreground-muted">{{ $t('target.detail.timeout') }}</div>
                      <div class="text-foreground">
                        {{ (selectedItem as RequestSnapshot).timeout_ms ?? '-' }} ms
                      </div>
                    </div>
                  </template>
                  <template v-else-if="isRunningSelected">
                    <div>
                      <div class="text-foreground-muted">{{ $t('target.detail.status') }}</div>
                      <div class="text-foreground">{{ $t('target.status.running') }}</div>
                    </div>
                    <div>
                      <div class="text-foreground-muted">{{ $t('target.detail.queuedFor') }}</div>
                      <div class="text-foreground">{{ (selectedItem as RunningSnapshot).queued_for_secs }}s</div>
                    </div>
                  </template>
                  <template v-else>
                    <div>
                      <div class="text-foreground-muted">{{ $t('target.detail.summary') }}</div>
                      <div class="text-foreground">{{ formatSummary(selectedItem as ResultSnapshot) }}</div>
                    </div>
                    <div>
                      <div class="text-foreground-muted">{{ $t('target.detail.queuedFor') }}</div>
                      <div class="text-foreground">{{ (selectedItem as ResultSnapshot).queued_for_secs }}s</div>
                    </div>
                  </template>
                </div>
              </div>

              <div v-if="isPendingSelected" class="flex flex-col gap-2">
                <button
                  class="flex items-center gap-2 bg-success hover:bg-success/90 text-white px-4 py-2 rounded shadow"
                  @click="emit('approve', selectedItem.id)"
                >
                  {{ $t('target.action.approve') }}
                  <span class="bg-success/50 px-1.5 rounded text-xs font-mono">{{ formatShortcut(props.settings.shortcuts.approve) }}</span>
                </button>
                <button
                  class="flex items-center gap-2 bg-danger hover:bg-danger/90 text-white px-4 py-2 rounded shadow"
                  @click="emit('deny', selectedItem.id)"
                >
                  {{ $t('target.action.deny') }}
                  <span class="bg-danger/50 px-1.5 rounded text-xs font-mono">{{ formatShortcut(props.settings.shortcuts.deny) }}</span>
                </button>
              </div>
              <div v-else-if="isRunningSelected" class="flex flex-col gap-2">
                <button
                  class="flex items-center gap-2 bg-danger hover:bg-danger/90 text-white px-4 py-2 rounded shadow"
                  @click="emit('cancel', selectedItem.id)"
                >
                  {{ $t('target.action.cancel') }}
                </button>
              </div>
            </div>

            <div class="flex-1 flex flex-col overflow-hidden">
              <div class="flex items-center justify-between px-6 py-2 bg-panel/80 border-b border-border">
                <span class="text-xs font-semibold text-foreground-muted uppercase">
                  {{ isPendingSelected ? $t('target.preview.pending') : $t('target.preview.output') }}
                </span>
                <button
                  class="text-foreground-muted hover:text-foreground p-1 rounded hover:bg-panel-muted transition-colors"
                  @click="isFullScreen = !isFullScreen"
                  :aria-label="isFullScreen ? $t('target.fullscreen.exit') : $t('target.fullscreen.enter')"
                  :title="isFullScreen ? $t('target.fullscreen.exit') : $t('target.fullscreen.enter')"
                >
                  <svg
                    v-if="!isFullScreen"
                    class="h-4 w-4"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <polyline points="15 3 21 3 21 9" />
                    <polyline points="9 21 3 21 3 15" />
                    <line x1="21" y1="3" x2="14" y2="10" />
                    <line x1="3" y1="21" x2="10" y2="14" />
                  </svg>
                  <svg
                    v-else
                    class="h-4 w-4"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <line x1="3" y1="3" x2="9" y2="9" />
                    <polyline points="9 5 9 9 5 9" />
                    <line x1="21" y1="3" x2="15" y2="9" />
                    <polyline points="15 5 15 9 19 9" />
                    <line x1="3" y1="21" x2="9" y2="15" />
                    <polyline points="9 19 9 15 5 15" />
                    <line x1="21" y1="21" x2="15" y2="15" />
                    <polyline points="15 19 15 15 19 15" />
                  </svg>
                </button>
              </div>
              <div class="flex-1 overflow-y-auto p-6 font-mono text-sm text-foreground whitespace-pre-wrap bg-panel-muted/40">
                <span v-if="!isPendingSelected && !isRunningSelected">
                  {{ buildOutput(selectedItem as ResultSnapshot) || $t('target.output.empty') }}
                </span>
                <span v-else-if="isRunningSelected" class="text-foreground-muted">{{ $t('target.output.running') }}</span>
                <span v-else class="text-foreground-muted">{{ $t('target.output.pending') }}</span>
              </div>
            </div>
          </template>

          <div v-else class="flex-1 flex items-center justify-center text-foreground-muted">
            {{ $t('target.emptySelection') }}
          </div>
        </div>
      </div>
      <div
        v-show="props.terminalOpen"
        class="h-2 bg-surface border-t border-border cursor-row-resize"
        :class="isResizing ? 'bg-accent/30 border-accent/40' : 'hover:bg-panel/80'"
        @mousedown.prevent="startResize"
      ></div>
      <div
        v-show="props.terminalOpen"
        class="flex-shrink-0 min-h-0 relative overflow-hidden"
        :style="terminalStyle"
        ref="terminalContainerRef"
      >
        <slot name="terminal" />
      </div>
    </div>

  </div>
</template>
