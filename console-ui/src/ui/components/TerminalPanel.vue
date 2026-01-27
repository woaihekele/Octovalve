<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch, withDefaults } from 'vue';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { useI18n } from 'vue-i18n';
import { terminalClose, terminalInput, terminalOpen, terminalResize } from '../../services/api';
import { formatErrorForUser } from '../../services/errors';
import { resolveTerminalTheme } from '../../shared/terminalTheme';
import type { TargetInfo } from '../../shared/types';
import type { ResolvedTheme } from '../../shared/theme';

const props = withDefaults(defineProps<{
  target: TargetInfo;
  visible: boolean;
  theme: ResolvedTheme;
  terminalScale?: number;
}>(), {
  terminalScale: 1,
});

const containerRef = ref<HTMLDivElement | null>(null);
const statusMessage = ref<string | null>(null);
const { t } = useI18n();

let terminal: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let sessionId: string | null = null;
let resizeObserver: ResizeObserver | null = null;
let unlistenOutput: UnlistenFn | null = null;
let unlistenExit: UnlistenFn | null = null;
let unlistenError: UnlistenFn | null = null;
let inputBuffer = '';
let inputFlushTimer: number | null = null;
let resizeIdleTimer: number | null = null;
// Resizing too aggressively can corrupt full-screen TUI apps (e.g. `top`) due to frequent
// local reflow + remote PTY resize while the app is emitting cursor-control sequences.
const RESIZE_THROTTLE_MS = 120;
const RESIZE_TRAIL_MS = 200;
let lastResizeFitAt = 0;
let lastRemoteCols = 0;
let lastRemoteRows = 0;

function canMeasureTerminal() {
  if (!props.visible || !containerRef.value) {
    return false;
  }
  // When the panel is hidden via `v-show`, the element is `display:none` and returns 0 sizes.
  const rect = containerRef.value.getBoundingClientRect();
  return rect.width >= 50 && rect.height >= 50;
}

function focusTerminal() {
  const focusable = containerRef.value?.querySelector('textarea');
  focusable?.focus();
}

function blurTerminal() {
  const focusable = containerRef.value?.querySelector('textarea');
  focusable?.blur();
}

function hasTerminalFocus() {
  const active = document.activeElement;
  return Boolean(active && containerRef.value && active instanceof Node && containerRef.value.contains(active));
}

defineExpose({
  focus: focusTerminal,
  blur: blurTerminal,
  hasFocus: hasTerminalFocus,
});

const termName = 'xterm-256color';
const BASE_TERMINAL_FONT_SIZE = 12;
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();

function resolveTerminalFontSize() {
  return BASE_TERMINAL_FONT_SIZE * (props.terminalScale || 1);
}

function applyTerminalTheme() {
  if (!terminal) {
    return;
  }
  terminal.options.theme = resolveTerminalTheme(props.theme);
  if (terminal.rows > 0) {
    terminal.refresh(0, terminal.rows - 1);
  }
}

function applyTerminalScale() {
  if (!terminal) {
    return;
  }
  terminal.options.fontSize = resolveTerminalFontSize();
  fitAddon?.fit();
  if (terminal.rows > 0) {
    terminal.refresh(0, terminal.rows - 1);
  }
  if (sessionId) {
    lastRemoteCols = terminal.cols;
    lastRemoteRows = terminal.rows;
    void terminalResize(sessionId, terminal.cols, terminal.rows);
  }
}

function runResizeFit(sendRemoteResize: boolean) {
  if (!terminal || !fitAddon || !canMeasureTerminal()) {
    return;
  }
  const beforeCols = terminal.cols;
  const beforeRows = terminal.rows;
  fitAddon.fit();
  const cols = terminal.cols;
  const rows = terminal.rows;
  if (!sendRemoteResize || !sessionId) {
    lastResizeFitAt = Date.now();
    return;
  }
  // Only resize the remote PTY when size actually changed, and preferably after the user stops resizing.
  if (cols === lastRemoteCols && rows === lastRemoteRows) {
    lastResizeFitAt = Date.now();
    return;
  }
  if (cols === beforeCols && rows === beforeRows) {
    lastResizeFitAt = Date.now();
    return;
  }
  lastRemoteCols = cols;
  lastRemoteRows = rows;
  void terminalResize(sessionId, cols, rows);
  lastResizeFitAt = Date.now();
}

function scheduleResizeFit() {
  if (!canMeasureTerminal()) {
    return;
  }
  if (resizeIdleTimer !== null) {
    window.clearTimeout(resizeIdleTimer);
  }
  resizeIdleTimer = window.setTimeout(() => {
    resizeIdleTimer = null;
    runResizeFit(true);
  }, RESIZE_TRAIL_MS);
  if (Date.now() - lastResizeFitAt >= RESIZE_THROTTLE_MS) {
    // Immediate local fit for responsiveness; defer remote PTY resize to the trailing edge.
    runResizeFit(false);
  }
}

function encodeBase64(bytes: Uint8Array) {
  let binary = '';
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary);
}

function decodeBase64(base64: string) {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

async function openSession() {
  if (sessionId) {
    return;
  }
  if (!props.target.terminal_available) {
    statusMessage.value = t('terminal.noSsh');
    return;
  }
  if (!containerRef.value) {
    statusMessage.value = t('terminal.containerNotReady');
    return;
  }

  terminal = new Terminal({
    cursorBlink: true,
    fontSize: resolveTerminalFontSize(),
    fontFamily: 'Menlo, Monaco, "Courier New", monospace',
    theme: resolveTerminalTheme(props.theme),
    scrollback: 5000,
  });
  fitAddon = new FitAddon();
  terminal.loadAddon(fitAddon);
  terminal.open(containerRef.value);
  fitAddon.fit();
  terminal.focus();

  const cols = terminal.cols;
  const rows = terminal.rows;
  statusMessage.value = t('terminal.connecting');
  try {
    sessionId = await terminalOpen(props.target.name, cols, rows, termName);
  } catch (err) {
    statusMessage.value = t('terminal.connectFailed', { error: formatErrorForUser(err, t) });
    cleanupTerminal(false);
    return;
  }

  terminal.onData((data) => {
    if (!sessionId) {
      return;
    }
    inputBuffer += data;
    scheduleInputFlush();
  });

  unlistenOutput = await listen('terminal_output', (event) => {
    const payload = event.payload as { session_id: string; data: string };
    if (payload.session_id !== sessionId || !terminal) {
      return;
    }
    const bytes = decodeBase64(payload.data);
    terminal.write(textDecoder.decode(bytes, { stream: true }));
  });

  unlistenExit = await listen('terminal_exit', (event) => {
    const payload = event.payload as { session_id: string; code?: number | null };
    if (payload.session_id !== sessionId) {
      return;
    }
    statusMessage.value = t('terminal.exited');
    cleanupTerminal(false);
  });

  unlistenError = await listen('terminal_error', (event) => {
    const payload = event.payload as { session_id: string; message: string };
    if (payload.session_id !== sessionId) {
      return;
    }
    statusMessage.value = payload.message || t('terminal.error');
    cleanupTerminal(false);
  });

  resizeObserver = new ResizeObserver(() => {
    // Ignore size changes while the panel is hidden; fitting in a collapsed box
    // will make xterm compute tiny geometry and may cause TUI corruption.
    if (!canMeasureTerminal()) {
      return;
    }
    scheduleResizeFit();
  });
  resizeObserver.observe(containerRef.value);

  statusMessage.value = null;
  lastRemoteCols = cols;
  lastRemoteRows = rows;
}

function scheduleInputFlush() {
  if (inputFlushTimer !== null) {
    return;
  }
  inputFlushTimer = window.setTimeout(() => {
    inputFlushTimer = null;
    flushInputBuffer();
  }, 12);
}

function flushInputBuffer() {
  if (!sessionId || inputBuffer.length === 0) {
    inputBuffer = '';
    return;
  }
  const bytes = textEncoder.encode(inputBuffer);
  inputBuffer = '';
  const payload = encodeBase64(bytes);
  void terminalInput(sessionId, payload);
}

async function syncTerminalLayout() {
  await nextTick();
  await new Promise<void>((resolve) => {
    window.requestAnimationFrame(() => resolve());
  });
  await new Promise<void>((resolve) => {
    window.requestAnimationFrame(() => resolve());
  });
  if (!terminal || !fitAddon || !sessionId) {
    return;
  }
  terminal.clearSelection();
  fitAddon.fit();
  if (terminal.rows > 0) {
    terminal.refresh(0, terminal.rows - 1);
  }
  lastRemoteCols = terminal.cols;
  lastRemoteRows = terminal.rows;
  void terminalResize(sessionId, terminal.cols, terminal.rows);
}

function cleanupTerminal(sendClose: boolean) {
  // Flush any pending UTF-8 decoder state between sessions.
  try {
    textDecoder.decode(new Uint8Array(), { stream: false });
  } catch {
    // ignore
  }
  if (inputFlushTimer !== null) {
    window.clearTimeout(inputFlushTimer);
    inputFlushTimer = null;
  }
  if (resizeIdleTimer !== null) {
    window.clearTimeout(resizeIdleTimer);
    resizeIdleTimer = null;
  }
  inputBuffer = '';
  if (resizeObserver && containerRef.value) {
    resizeObserver.disconnect();
    resizeObserver = null;
  }
  if (unlistenOutput) {
    unlistenOutput();
    unlistenOutput = null;
  }
  if (unlistenExit) {
    unlistenExit();
    unlistenExit = null;
  }
  if (unlistenError) {
    unlistenError();
    unlistenError = null;
  }
  if (terminal) {
    terminal.dispose();
    terminal = null;
  }
  if (fitAddon) {
    fitAddon.dispose();
    fitAddon = null;
  }
  if (sessionId) {
    if (sendClose) {
      void terminalClose(sessionId);
    }
    sessionId = null;
  }
}

watch(
  () => props.target.name,
  () => {
    cleanupTerminal(true);
    void openSession();
  }
);

watch(
  () => props.theme,
  () => {
    applyTerminalTheme();
  }
);

watch(
  () => props.terminalScale,
  () => {
    applyTerminalScale();
  },
  { flush: 'post' }
);

watch(
  () => props.visible,
  async (visible) => {
    if (!visible) {
      if (resizeIdleTimer !== null) {
        window.clearTimeout(resizeIdleTimer);
        resizeIdleTimer = null;
      }
      if (terminal) {
        terminal.clearSelection();
        terminal.blur();
      }
      return;
    }
    if (!sessionId) {
      await openSession();
    }
    await syncTerminalLayout();
  },
  { flush: 'post' }
);

onMounted(() => {
  if (props.visible) {
    void openSession().then(() => {
      void syncTerminalLayout();
    });
  }
});

onBeforeUnmount(() => {
  cleanupTerminal(true);
});
</script>

<template>
  <div class="absolute inset-0 flex flex-col bg-surface">
    <div class="flex-1 min-h-0">
      <div ref="containerRef" class="h-full w-full" />
    </div>
    <div v-if="statusMessage" class="px-4 py-2 text-xs text-warning bg-panel/60 border-t border-border">
      {{ statusMessage }}
    </div>
  </div>
</template>

<style scoped>
:deep(.xterm-viewport) {
  scrollbar-color: rgb(var(--color-scrollbar)) rgb(var(--color-scrollbar-track));
}

:deep(.xterm-viewport::-webkit-scrollbar) {
  width: 10px;
}

:deep(.xterm-viewport::-webkit-scrollbar-track) {
  background: rgb(var(--color-scrollbar-track));
}

:deep(.xterm-viewport::-webkit-scrollbar-thumb) {
  background-color: rgb(var(--color-scrollbar));
  border-radius: 8px;
  border: 2px solid rgb(var(--color-scrollbar-track));
}

:deep(.xterm-viewport::-webkit-scrollbar-thumb:hover) {
  background-color: rgb(var(--color-scrollbar));
}
</style>
