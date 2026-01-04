<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Terminal, type ITheme } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { useI18n } from 'vue-i18n';
import { terminalClose, terminalInput, terminalOpen, terminalResize } from '../../services/api';
import type { TargetInfo } from '../../shared/types';
import type { ResolvedTheme } from '../../shared/theme';

const props = defineProps<{
  target: TargetInfo;
  visible: boolean;
  theme: ResolvedTheme;
}>();

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
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();
function buildExtendedAnsi(overrides: Record<number, string>): string[] {
  const colors: string[] = [];
  for (const [index, value] of Object.entries(overrides)) {
    const parsed = Number(index);
    if (Number.isNaN(parsed) || parsed < 16 || parsed > 255) {
      continue;
    }
    colors[parsed - 16] = value;
  }
  return colors;
}

const EXTENDED_ANSI_LIGHT = buildExtendedAnsi({
  3: '#9a6700',
  5: '#8250df',
  9: '#cf222e',
  10: '#2da44e',
  11: '#bf8700',
  13: '#8250df',
  15: '#ffffff',
  16: '#24292f',
  21: '#0969da',
  27: '#0969da',
  34: '#1a7f37',
  45: '#1b7c83',
  51: '#3192aa',
  196: '#a40e26',
  226: '#bf8700',
  232: '#24292f',
});

const EXTENDED_ANSI_DARK = buildExtendedAnsi({
  3: '#d29922',
  5: '#bc8cff',
  9: '#ff7b72',
  10: '#3fb950',
  11: '#e3b341',
  13: '#bc8cff',
  15: '#f0f6fc',
  16: '#0d1117',
  21: '#1f6feb',
  27: '#58a6ff',
  34: '#3fb950',
  45: '#39c5cf',
  51: '#56d4dd',
  196: '#da3633',
  226: '#e3b341',
  232: '#161b22',
});

const GITHUB_LIGHT_THEME: ITheme = {
  background: '#ffffff',
  foreground: '#24292f',
  cursor: '#8250df',
  cursorAccent: '#ffffff',
  selectionBackground: 'rgba(130, 80, 223, 0.22)',
  selectionInactiveBackground: 'rgba(130, 80, 223, 0.16)',
  selectionForeground: '#24292f',
  black: '#24292f',
  red: '#cf222e',
  green: '#1a7f37',
  yellow: '#9a6700',
  blue: '#0969da',
  magenta: '#8250df',
  cyan: '#1b7c83',
  white: '#d0d7de',
  brightBlack: '#57606a',
  brightRed: '#a40e26',
  brightGreen: '#2da44e',
  brightYellow: '#bf8700',
  brightBlue: '#218bff',
  brightMagenta: '#a475f9',
  brightCyan: '#3192aa',
  brightWhite: '#ffffff',
  extendedAnsi: EXTENDED_ANSI_LIGHT,
};

const DARCULA_THEME: ITheme = {
  background: '#2B2B2B',
  foreground: '#A9B7C6',
  cursor: '#A9B7C6',
  cursorAccent: '#2B2B2B',
  selectionBackground: 'rgba(104, 151, 187, 0.35)',
  selectionInactiveBackground: 'rgba(104, 151, 187, 0.22)',
  selectionForeground: '#A9B7C6',
  black: '#000000',
  red: '#cc666e',
  green: '#6a8759',
  yellow: '#bbb529',
  blue: '#6897bb',
  magenta: '#9876aa',
  cyan: '#6d9cbe',
  white: '#a9b7c6',
  brightBlack: '#4e5254',
  brightRed: '#d46a6a',
  brightGreen: '#87af5f',
  brightYellow: '#d0d050',
  brightBlue: '#7aa6c2',
  brightMagenta: '#b294bb',
  brightCyan: '#83b7c9',
  brightWhite: '#ffffff',
};

const GITHUB_DARK_THEME: ITheme = {
  background: '#0d1117',
  foreground: '#c9d1d9',
  cursor: '#8957e5',
  cursorAccent: '#0d1117',
  selectionBackground: 'rgba(137, 87, 229, 0.35)',
  selectionInactiveBackground: 'rgba(137, 87, 229, 0.22)',
  selectionForeground: '#c9d1d9',
  black: '#484f58',
  red: '#ff7b72',
  green: '#3fb950',
  yellow: '#d29922',
  blue: '#58a6ff',
  magenta: '#bc8cff',
  cyan: '#39c5cf',
  white: '#b1bac4',
  brightBlack: '#6e7681',
  brightRed: '#ffa198',
  brightGreen: '#56d364',
  brightYellow: '#e3b341',
  brightBlue: '#79c0ff',
  brightMagenta: '#d2a8ff',
  brightCyan: '#56d4dd',
  brightWhite: '#f0f6fc',
  extendedAnsi: EXTENDED_ANSI_DARK,
};

function resolveTerminalTheme() {
  if (props.theme === 'light') {
    return GITHUB_LIGHT_THEME;
  }
  if (props.theme === 'darcula') {
    return DARCULA_THEME;
  }
  return GITHUB_DARK_THEME;
}

function applyTerminalTheme() {
  if (!terminal) {
    return;
  }
  terminal.options.theme = resolveTerminalTheme();
  if (terminal.rows > 0) {
    terminal.refresh(0, terminal.rows - 1);
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
    fontSize: 12,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace',
    theme: resolveTerminalTheme(),
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
    statusMessage.value = t('terminal.connectFailed', { error: String(err) });
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
    terminal.write(textDecoder.decode(bytes));
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
    if (!terminal || !fitAddon || !sessionId) {
      return;
    }
    fitAddon.fit();
    void terminalResize(sessionId, terminal.cols, terminal.rows);
  });
  resizeObserver.observe(containerRef.value);

  statusMessage.value = null;
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
  void terminalResize(sessionId, terminal.cols, terminal.rows);
}

function cleanupTerminal(sendClose: boolean) {
  if (inputFlushTimer !== null) {
    window.clearTimeout(inputFlushTimer);
    inputFlushTimer = null;
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
  () => props.visible,
  async (visible) => {
    if (!visible) {
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
