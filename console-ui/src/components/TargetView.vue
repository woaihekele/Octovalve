<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { formatShortcut, matchesShortcut } from '../shortcuts';
import { startWindowDrag } from '../tauriWindow';
import type { AppSettings, ListTab, RequestSnapshot, ResultSnapshot, ServiceSnapshot, TargetInfo } from '../types';
import TerminalPanel from './TerminalPanel.vue';

const props = defineProps<{
  target: TargetInfo;
  snapshot: ServiceSnapshot | null;
  settings: AppSettings;
  pendingJumpToken: number;
}>();

const emit = defineEmits<{
  (e: 'approve', id: string): void;
  (e: 'deny', id: string): void;
}>();

const activeTab = ref<ListTab>('pending');
const selectedIndex = ref(0);
const isFullScreen = ref(false);
const isTerminalOpen = ref(false);
const isTerminalInitialized = ref(false);

const pendingList = computed(() => props.snapshot?.queue ?? []);
const historyList = computed(() => props.snapshot?.history ?? []);
const currentList = computed(() => (activeTab.value === 'pending' ? pendingList.value : historyList.value));
const selectedItem = computed(() => currentList.value[selectedIndex.value] ?? null);

watch(
  () => [props.target.name, activeTab.value],
  () => {
    selectedIndex.value = 0;
    isTerminalOpen.value = false;
    isTerminalInitialized.value = false;
  }
);

watch(
  () => props.pendingJumpToken,
  () => {
    activeTab.value = 'pending';
    selectedIndex.value = 0;
  }
);

watch(
  () => currentList.value.length,
  (length) => {
    if (length === 0) {
      selectedIndex.value = 0;
      return;
    }
    if (selectedIndex.value >= length) {
      selectedIndex.value = length - 1;
    }
  }
);

function formatTime(value: number) {
  return new Date(value).toLocaleString();
}

function formatPipeline(item: RequestSnapshot | ResultSnapshot) {
  return item.pipeline.map((stage) => stage.argv.join(' ')).join(' | ');
}

function formatSummary(result: ResultSnapshot) {
  if (result.status === 'completed') {
    return `completed (exit=${result.exit_code ?? 'n/a'})`;
  }
  if (result.status === 'denied') {
    return 'denied';
  }
  if (result.status === 'error') {
    return 'error';
  }
  return result.status;
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
  if (isTerminalOpen.value) {
    return;
  }

  const key = event.key;

  if (key === 'j' || key === 'ArrowDown') {
    event.preventDefault();
    selectedIndex.value = Math.min(selectedIndex.value + 1, currentList.value.length - 1);
    return;
  }

  if (key === 'k' || key === 'ArrowUp') {
    event.preventDefault();
    selectedIndex.value = Math.max(selectedIndex.value - 1, 0);
    return;
  }

  if (matchesShortcut(event, props.settings.shortcuts.toggleList)) {
    event.preventDefault();
    activeTab.value = activeTab.value === 'pending' ? 'history' : 'pending';
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

  if (activeTab.value === 'pending' && selectedItem.value) {
    if (matchesShortcut(event, props.settings.shortcuts.approve)) {
      emit('approve', selectedItem.value.id);
    } else if (matchesShortcut(event, props.settings.shortcuts.deny)) {
      emit('deny', selectedItem.value.id);
    }
  }
}

onMounted(() => window.addEventListener('keydown', handleKeyDown));
onBeforeUnmount(() => window.removeEventListener('keydown', handleKeyDown));

function handleTitleDrag(event: MouseEvent) {
  if (event.button !== 0) {
    return;
  }
  event.preventDefault();
  void startWindowDrag();
}

function openTerminal() {
  if (!props.target.terminal_available) {
    return;
  }
  isTerminalInitialized.value = true;
  isTerminalOpen.value = true;
}
</script>

<template>
  <div class="flex flex-col h-full bg-slate-950" :class="isFullScreen ? 'fixed inset-0 z-40' : 'relative'">
    <div
      v-if="!isFullScreen"
      class="h-16 border-b border-slate-800 flex items-center justify-between px-6 bg-slate-900/50"
      data-tauri-drag-region
      @mousedown="handleTitleDrag"
    >
      <div class="flex items-center gap-4">
        <div>
          <h2 class="text-xl font-semibold text-slate-100">{{ props.target.name }}</h2>
          <div class="flex items-center gap-2 text-sm text-slate-500">
            <span>{{ props.target.desc }}</span>
            <span class="w-1 h-1 bg-slate-600 rounded-full"></span>
            <span>{{ props.target.hostname || props.target.ip || 'unknown' }}</span>
          </div>
        </div>
      </div>
      <div class="flex items-center gap-2">
        <button
          class="p-2 rounded border transition-colors"
          :class="
            props.target.terminal_available
              ? isTerminalOpen
                ? 'bg-indigo-500/20 text-indigo-200 border-indigo-500/40'
                : 'bg-slate-900/60 text-slate-200 border-slate-700 hover:border-indigo-500/40'
              : 'bg-slate-900/30 text-slate-500 border-slate-800 cursor-not-allowed'
          "
          :disabled="!props.target.terminal_available"
          @click="openTerminal"
          aria-label="终端"
          title="终端"
        >
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="4" width="18" height="16" rx="2"></rect>
            <polyline points="8 9 11 12 8 15"></polyline>
            <line x1="13" y1="15" x2="17" y2="15"></line>
          </svg>
        </button>
      </div>
    </div>

    <div class="flex-1 flex overflow-hidden min-h-0">
      <div v-if="!isFullScreen" class="w-1/3 min-w-[320px] border-r border-slate-800 flex flex-col bg-slate-900/20 min-h-0">
        <div class="flex border-b border-slate-800">
          <button
            class="flex-1 py-3 text-sm font-medium transition-colors"
            :class="activeTab === 'pending' ? 'text-indigo-300 border-b-2 border-indigo-400 bg-indigo-400/5' : 'text-slate-500 hover:text-slate-300'"
            @click="activeTab = 'pending'"
          >
            Pending <span class="ml-1 text-xs bg-slate-800 px-1.5 py-0.5 rounded-full text-slate-300">{{ pendingList.length }}</span>
          </button>
          <button
            class="flex-1 py-3 text-sm font-medium transition-colors"
            :class="activeTab === 'history' ? 'text-indigo-300 border-b-2 border-indigo-400 bg-indigo-400/5' : 'text-slate-500 hover:text-slate-300'"
            @click="activeTab = 'history'"
          >
            History <span class="ml-1 text-xs bg-slate-800 px-1.5 py-0.5 rounded-full text-slate-300">{{ historyList.length }}</span>
          </button>
        </div>

        <div class="flex-1 overflow-y-auto min-h-0">
          <div v-if="currentList.length === 0" class="p-8 text-center text-slate-600 text-sm">
            暂无 {{ activeTab === 'pending' ? 'Pending' : 'History' }} 记录。
          </div>
          <div
            v-for="(item, index) in currentList"
            :key="item.id"
            class="p-4 border-b border-slate-800 cursor-pointer transition-colors"
            :class="index === selectedIndex ? 'bg-indigo-900/20 border-l-4 border-l-indigo-400' : 'hover:bg-slate-800/30 border-l-4 border-l-transparent'"
            @click="selectedIndex = index"
          >
            <div class="flex justify-between items-start mb-1">
              <span class="font-mono text-sm line-clamp-1" :class="index === selectedIndex ? 'text-indigo-300' : 'text-slate-300'">
                {{ item.raw_command }}
              </span>
              <span
                v-if="activeTab === 'history'"
                class="text-xs px-2 py-0.5 rounded"
                :class="(item as ResultSnapshot).status === 'completed' ? 'bg-emerald-500/20 text-emerald-300' : (item as ResultSnapshot).status === 'denied' ? 'bg-rose-500/20 text-rose-300' : 'bg-amber-500/20 text-amber-300'"
              >
                {{ (item as ResultSnapshot).status }}
              </span>
            </div>
            <div class="flex justify-between items-center text-xs text-slate-500">
              <span>
                {{ activeTab === 'pending' ? formatTime((item as RequestSnapshot).received_at_ms) : formatTime((item as ResultSnapshot).finished_at_ms) }}
              </span>
              <span v-if="activeTab === 'pending'" class="truncate">{{ (item as RequestSnapshot).intent }}</span>
            </div>
          </div>
        </div>
      </div>

      <div class="flex-1 flex flex-col">
        <template v-if="selectedItem">
          <div class="border-b border-slate-800 bg-slate-900/30 p-6 flex justify-between gap-6">
            <div class="flex-1">
              <h3 class="text-xs font-semibold text-slate-400 uppercase tracking-wider mb-2">Command</h3>
              <code class="block text-base text-indigo-200 font-mono bg-slate-900 px-4 py-3 rounded-lg border border-slate-800">
                {{ selectedItem.raw_command }}
              </code>

              <div class="mt-4 grid grid-cols-2 gap-4 text-xs text-slate-400">
                <div>
                  <div class="text-slate-500">Intent</div>
                  <div class="text-slate-200">{{ selectedItem.intent }}</div>
                </div>
                <div>
                  <div class="text-slate-500">Mode</div>
                  <div class="text-slate-200">{{ selectedItem.mode }}</div>
                </div>
                <div>
                  <div class="text-slate-500">CWD</div>
                  <div class="text-slate-200">{{ selectedItem.cwd || '-' }}</div>
                </div>
                <div>
                  <div class="text-slate-500">Peer</div>
                  <div class="text-slate-200">{{ selectedItem.peer }}</div>
                </div>
                <div>
                  <div class="text-slate-500">Pipeline</div>
                  <div class="text-slate-200">{{ formatPipeline(selectedItem) || '-' }}</div>
                </div>
                <template v-if="activeTab === 'pending'">
                  <div>
                    <div class="text-slate-500">Timeout</div>
                    <div class="text-slate-200">{{ (selectedItem as RequestSnapshot).timeout_ms ?? '-' }} ms</div>
                  </div>
                </template>
                <template v-else>
                  <div>
                    <div class="text-slate-500">Summary</div>
                    <div class="text-slate-200">{{ formatSummary(selectedItem as ResultSnapshot) }}</div>
                  </div>
                  <div>
                    <div class="text-slate-500">Queued For</div>
                    <div class="text-slate-200">{{ (selectedItem as ResultSnapshot).queued_for_secs }}s</div>
                  </div>
                </template>
              </div>
            </div>

            <div v-if="activeTab === 'pending'" class="flex flex-col gap-2">
              <button
                class="flex items-center gap-2 bg-emerald-600 hover:bg-emerald-500 text-white px-4 py-2 rounded shadow"
                @click="emit('approve', selectedItem.id)"
              >
                Approve <span class="bg-emerald-700/50 px-1.5 rounded text-xs font-mono">{{ formatShortcut(props.settings.shortcuts.approve) }}</span>
              </button>
              <button
                class="flex items-center gap-2 bg-rose-600 hover:bg-rose-500 text-white px-4 py-2 rounded shadow"
                @click="emit('deny', selectedItem.id)"
              >
                Deny <span class="bg-rose-700/50 px-1.5 rounded text-xs font-mono">{{ formatShortcut(props.settings.shortcuts.deny) }}</span>
              </button>
            </div>
          </div>

          <div class="flex-1 flex flex-col overflow-hidden">
            <div class="flex items-center justify-between px-6 py-2 bg-slate-900/80 border-b border-slate-800">
              <span class="text-xs font-semibold text-slate-500 uppercase">
                {{ activeTab === 'history' ? 'Execution Output' : 'Pending Preview' }}
              </span>
              <button
                class="text-slate-400 hover:text-white p-1 rounded hover:bg-slate-800 transition-colors"
                @click="isFullScreen = !isFullScreen"
                :aria-label="isFullScreen ? '退出全屏' : '全屏'"
                :title="isFullScreen ? '退出全屏' : '全屏'"
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
            <div class="flex-1 overflow-y-auto p-6 font-mono text-sm text-slate-200 whitespace-pre-wrap bg-black/40">
              <span v-if="activeTab === 'history'">
                {{ buildOutput(selectedItem as ResultSnapshot) || '无输出' }}
              </span>
              <span v-else class="text-slate-500">等待审批后输出将出现在此处。</span>
            </div>
          </div>
        </template>

        <div v-else class="flex-1 flex items-center justify-center text-slate-600">
          请选择一条记录查看详情
        </div>
      </div>
    </div>

    <TerminalPanel
      v-if="isTerminalInitialized"
      v-show="isTerminalOpen"
      :target="props.target"
      :visible="isTerminalOpen"
      @close="isTerminalOpen = false"
    />
  </div>
</template>
