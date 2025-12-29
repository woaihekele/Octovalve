<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Terminal } from 'xterm';
import { FitAddon } from 'xterm-addon-fit';
import 'xterm/css/xterm.css';
import { terminalClose, terminalInput, terminalOpen, terminalResize } from '../api';
import type { TargetInfo } from '../types';

const props = defineProps<{
  target: TargetInfo;
  visible: boolean;
}>();

const emit = defineEmits<{
  (e: 'close'): void;
}>();

const containerRef = ref<HTMLDivElement | null>(null);
const statusMessage = ref<string | null>(null);

let terminal: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let sessionId: string | null = null;
let resizeObserver: ResizeObserver | null = null;
let unlistenOutput: UnlistenFn | null = null;
let unlistenExit: UnlistenFn | null = null;
let unlistenError: UnlistenFn | null = null;
let inputBuffer = '';
let inputFlushTimer: number | null = null;

const termName = 'xterm-256color';
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();

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
    statusMessage.value = '该目标未配置 ssh，无法打开终端。';
    return;
  }
  if (!containerRef.value) {
    statusMessage.value = '终端容器尚未就绪。';
    return;
  }

  terminal = new Terminal({
    cursorBlink: true,
    fontSize: 12,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace',
    theme: {
      background: '#020617',
      foreground: '#e2e8f0',
    },
    scrollback: 5000,
  });
  fitAddon = new FitAddon();
  terminal.loadAddon(fitAddon);
  terminal.open(containerRef.value);
  fitAddon.fit();
  terminal.focus();

  const cols = terminal.cols;
  const rows = terminal.rows;
  statusMessage.value = '正在连接远端终端...';
  try {
    sessionId = await terminalOpen(props.target.name, cols, rows, termName);
  } catch (err) {
    statusMessage.value = `终端连接失败：${String(err)}`;
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
    statusMessage.value = '终端已退出。';
    cleanupTerminal(false);
  });

  unlistenError = await listen('terminal_error', (event) => {
    const payload = event.payload as { session_id: string; message: string };
    if (payload.session_id !== sessionId) {
      return;
    }
    statusMessage.value = payload.message || '终端连接异常。';
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

function handleClose() {
  emit('close');
}

watch(
  () => props.target.name,
  () => {
    cleanupTerminal(true);
    void openSession();
  }
);

watch(
  () => props.visible,
  (visible) => {
    if (!visible) {
      return;
    }
    if (!sessionId) {
      void openSession();
      return;
    }
    if (terminal && fitAddon && sessionId) {
      fitAddon.fit();
      terminal.focus();
      void terminalResize(sessionId, terminal.cols, terminal.rows);
    }
  }
);

onMounted(() => {
  if (props.visible) {
    void openSession();
  }
});

onBeforeUnmount(() => {
  cleanupTerminal(true);
});
</script>

<template>
  <div class="absolute inset-0 z-40 flex flex-col bg-slate-950">
    <div class="flex items-center justify-between px-4 py-2 border-b border-slate-800 bg-slate-900/70">
      <div class="text-sm text-slate-200">{{ props.target.name }} · 终端</div>
      <button
        class="p-1.5 text-slate-400 hover:text-white border border-slate-700 rounded"
        @click="handleClose"
        aria-label="关闭终端"
        title="关闭终端"
      >
        <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <line x1="18" y1="6" x2="6" y2="18" />
          <line x1="6" y1="6" x2="18" y2="18" />
        </svg>
      </button>
    </div>
    <div class="flex-1 min-h-0">
      <div ref="containerRef" class="h-full w-full" />
    </div>
    <div v-if="statusMessage" class="px-4 py-2 text-xs text-amber-300 bg-slate-900/60 border-t border-slate-800">
      {{ statusMessage }}
    </div>
  </div>
</template>
