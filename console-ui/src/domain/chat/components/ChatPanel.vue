<template>
  <div
    class="chat-panel"
    :class="{
      'chat-panel--open': isOpen,
      'chat-panel--resizing': isResizing,
      'chat-panel--drop-active': showDropHint,
    }"
    :style="panelStyle"
  >
    <div
      v-show="isOpen"
      class="chat-panel__resizer"
      @mousedown.prevent="startResize"
    ></div>
    <div v-if="showDropHint" class="chat-panel__drop-overlay">
      <div class="chat-panel__drop-hint">{{ $t('chat.dropHint') }}</div>
    </div>
    <div
      class="chat-panel__content"
      @dragenter.prevent="handlePanelDragOver"
      @dragover.prevent="handlePanelDragOver"
      @drop.prevent="handlePanelDrop"
    >
      <div class="chat-panel__header">
        <div class="chat-panel__header-left">
          <div class="chat-panel__header-info">
            <div class="chat-panel__title-row">
              <span class="chat-panel__title-text">{{ title }}</span>
              <span class="chat-panel__provider-badge">
                {{ provider === 'acp' ? $t('chat.provider.acpLabel') : $t('chat.provider.openaiLabel') }}
              </span>
            </div>
            <div class="chat-panel__status">
              <span class="chat-panel__status-dot" :class="{ 'chat-panel__status-dot--connected': isConnected }"></span>
              <span class="chat-panel__status-text">
                {{ isConnected ? $t('chat.status.connected') : $t('chat.status.disconnected') }}
              </span>
            </div>
          </div>
        </div>
        <div class="chat-panel__actions">
          <button
            class="chat-panel__btn"
            :title="$t('chat.history.title')"
            @click="$emit('show-history')"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M3 12a9 9 0 1 0 3-6.7" />
              <polyline points="3 4 3 10 9 10" />
            </svg>
          </button>
          <button class="chat-panel__btn" :title="$t('chat.clear')" @click="$emit('clear')">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
          </button>
        </div>
      </div>

      <div class="chat-panel__messages" ref="messagesRef" @scroll="handleScroll">
        <div ref="contentRef" class="chat-panel__messages-content">
          <ChatPlanCard v-if="planEntries.length > 0" :entries="planEntries" />
          <div v-if="messages.length === 0" class="chat-panel__welcome">
            <p>{{ $t('chat.welcome') }}</p>
          </div>
          <template v-else>
            <ChatMessageRow
              v-for="message in messages"
              :key="message.id"
              :message="message"
              :is-last="message.id === messages[messages.length - 1]?.id"
              @toggle-thinking="handleToggleThinking"
            />
          </template>
          <div ref="scrollAnchor" class="chat-panel__scroll-anchor"></div>
        </div>
      </div>

      <ChatInput
        ref="inputRef"
        v-model="inputValue"
        :placeholder="resolvedPlaceholder"
        :disabled="!isConnected || inputLocked"
        :is-streaming="isStreaming"
        :provider="provider"
        :supports-image="supportsImages"
        :send-on-enter="sendOnEnter"
        :targets="targets"
        @send="handleSend"
        @cancel="$emit('cancel')"
        @change-provider="$emit('change-provider', $event)"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, ref, watch, nextTick, computed } from 'vue';
import { useI18n } from 'vue-i18n';
import ChatMessageRow from './ChatMessageRow.vue';
import ChatInput from './ChatInput.vue';
import ChatPlanCard from './ChatPlanCard.vue';
import type { ChatMessage, PlanEntry, SendMessageOptions } from '../types';
import { useStickToBottom } from '../composables/useStickToBottom';
import type { TargetInfo } from '../../../shared/types';

interface Props {
  isOpen: boolean;
  title?: string;
  greeting?: string;
  placeholder?: string;
  width?: number;
  useStoredWidth?: boolean;
  messages: ChatMessage[];
  planEntries?: PlanEntry[];
  isStreaming?: boolean;
  isConnected?: boolean;
  inputLocked?: boolean;
  provider?: 'acp' | 'openai';
  sendOnEnter?: boolean;
  targets?: TargetInfo[];
  supportsImages?: boolean;
  showDropHint?: boolean;
}

const props = withDefaults(defineProps<Props>(), {
  width: 380,
  useStoredWidth: true,
  isStreaming: false,
  isConnected: true,
  inputLocked: false,
  provider: 'acp',
  sendOnEnter: false,
  planEntries: () => [],
  targets: () => [],
  supportsImages: false,
  showDropHint: false,
});

const emit = defineEmits<{
  send: [options: SendMessageOptions];
  cancel: [];
  'show-history': [];
  clear: [];
  'change-provider': [provider: 'acp' | 'openai'];
}>();

const widthStorageKey = 'console-ui.chat-panel.width';
const minPanelWidth = 320;
const maxPanelWidth = 720;
const { t } = useI18n();
function resolvePlatformShortcut() {
  if (typeof navigator === 'undefined') {
    return 'Cmd+Enter';
  }
  const platform =
    (navigator as { userAgentData?: { platform?: string } }).userAgentData?.platform ||
    navigator.platform ||
    navigator.userAgent;
  return /mac|iphone|ipad|ipod/i.test(platform) ? 'Cmd+Enter' : 'Ctrl+Enter';
}

const sendShortcutLabel = computed(() => resolvePlatformShortcut());

const resolvedPlaceholder = computed(() => {
  if (props.placeholder) {
    return props.placeholder;
  }
  return props.sendOnEnter
    ? t('chat.input.placeholder.sendOnEnter')
    : t('chat.input.placeholder.sendOnShortcut', { shortcut: sendShortcutLabel.value });
});

function clampWidth(value: number) {
  return Math.min(maxPanelWidth, Math.max(minPanelWidth, value));
}

function readStoredWidth() {
  if (typeof window === 'undefined') {
    return undefined;
  }
  const raw = window.localStorage.getItem(widthStorageKey);
  if (!raw) {
    return undefined;
  }
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed)) {
    return undefined;
  }
  return clampWidth(parsed);
}

const panelWidth = ref(
  props.useStoredWidth === false
    ? clampWidth(props.width ?? minPanelWidth)
    : (readStoredWidth() ?? clampWidth(props.width ?? minPanelWidth))
);
const isResizing = ref(false);

const panelStyle = computed(() => {
  return {
    ['--chat-panel-width']: `${panelWidth.value}px`,
  } as Record<string, string>;
});

let resizeStartX = 0;
let resizeStartWidth = 0;

function persistWidth() {
  if (typeof window === 'undefined') {
    return;
  }
  window.localStorage.setItem(widthStorageKey, String(panelWidth.value));
}

function handleResizeMove(event: MouseEvent) {
  const dx = resizeStartX - event.clientX;
  panelWidth.value = clampWidth(resizeStartWidth + dx);
}

function stopResize() {
  if (!isResizing.value) {
    return;
  }
  isResizing.value = false;
  window.removeEventListener('mousemove', handleResizeMove);
  window.removeEventListener('mouseup', stopResize);
  persistWidth();
}

function startResize(event: MouseEvent) {
  if (!props.isOpen) {
    return;
  }
  isResizing.value = true;
  resizeStartX = event.clientX;
  resizeStartWidth = panelWidth.value;
  window.addEventListener('mousemove', handleResizeMove);
  window.addEventListener('mouseup', stopResize);
}

const inputValue = ref('');
const messagesRef = ref<HTMLElement | null>(null);
const contentRef = ref<HTMLElement | null>(null);
const scrollAnchor = ref<HTMLElement | null>(null);
type ChatInputExpose = {
  addExternalFiles: (files: File[]) => void;
  focus: () => void;
};
const inputRef = ref<ChatInputExpose | null>(null);

const { stickToBottom, scrollToBottom, handleScroll, activateStickToBottom } = useStickToBottom(
  messagesRef,
  contentRef
);

function handleToggleThinking(opened: boolean) {
  if (opened && stickToBottom.value) {
    void scrollToBottom({ force: true, behavior: 'smooth' });
  }
}

function canAcceptDrop() {
  return props.isOpen && props.isConnected && !props.inputLocked && !props.isStreaming;
}

function handlePanelDragOver(event: DragEvent) {
  if (!canAcceptDrop()) {
    return;
  }
  event.dataTransfer?.setData('text/plain', '');
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = 'copy';
  }
}

function handlePanelDrop(event: DragEvent) {
  if (!canAcceptDrop()) {
    return;
  }
  const files = Array.from(event.dataTransfer?.files || []);
  if (files.length === 0) {
    return;
  }
  inputRef.value?.addExternalFiles(files);
}

onBeforeUnmount(() => {
  if (typeof window !== 'undefined') {
    window.removeEventListener('mousemove', handleResizeMove);
    window.removeEventListener('mouseup', stopResize);
  }
});

function handleSend(options: SendMessageOptions) {
  activateStickToBottom();
  emit('send', options);
  inputValue.value = '';
}

watch(
  () => props.messages.length,
  () => {
    nextTick(() => {
      void scrollToBottom();
    });
  }
);

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      nextTick(() => {
        activateStickToBottom();
        void scrollToBottom({ force: true, behavior: 'smooth' });
      });
    }
  }
);
</script>

<style scoped lang="scss">
.chat-panel {
  position: relative;
  height: 100%;
  width: 0;
  background: rgb(var(--color-panel));
  transition: width 0.25s ease;
  overflow: visible;
  flex-shrink: 0;

  &--open {
    width: var(--chat-panel-width);
    min-width: 320px;
  }

  &--resizing {
    transition: none;
    user-select: none;
  }

  &--drop-active {
    box-shadow: inset 0 0 0 1px rgba(var(--color-accent), 0.35);
  }

  &__drop-overlay {
    position: absolute;
    inset: 0;
    z-index: 4;
    background: rgba(0, 0, 0, 0.35);
    pointer-events: none;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  &__resizer {
    position: absolute;
    left: -4px;
    top: 0;
    bottom: 0;
    width: 8px;
    cursor: col-resize;
    z-index: 2;
    background: transparent;

    &::after {
      content: '';
      position: absolute;
      top: 0;
      bottom: 0;
      left: 50%;
      width: 1px;
      background: rgb(var(--color-border));
      transform: translateX(-50%);
      pointer-events: none;
    }

    &:hover {
      background: rgb(var(--color-accent) / 0.18);
    }
  }

  &--resizing &__resizer {
    background: rgb(var(--color-accent) / 0.18);
  }

  &--drop-active &__resizer::after {
    background: rgba(var(--color-accent), 0.6);
  }

  &__content {
    display: flex;
    flex-direction: column;
    height: 100%;
    width: var(--chat-panel-width);
    opacity: 0;
    transition: opacity 0.15s ease 0.1s;
    position: relative;
    overflow: hidden;
    pointer-events: none;

    .chat-panel--open & {
      opacity: 1;
      pointer-events: auto;
    }

    .chat-panel--resizing & {
      transition: none;
    }
  }

  &__drop-hint {
    padding: 10px 16px;
    border-radius: 8px;
    border: 1px solid rgba(var(--color-accent), 0.6);
    background: rgb(var(--color-panel));
    color: rgb(var(--color-text));
    font-size: 15px;
    text-align: center;
    font-weight: 600;
    box-shadow: 0 6px 12px rgba(0, 0, 0, 0.35);
    pointer-events: none;
  }

  &__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px;
    background: rgb(var(--color-panel));
    border-bottom: 1px solid rgb(var(--color-border));
    flex-shrink: 0;
  }

  &__header-left {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  &__avatar {
    width: 36px;
    height: 36px;
    background: rgba(255, 255, 255, 0.2);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(4px);
    border: 1px solid rgba(255, 255, 255, 0.15);
  }

  &__avatar-icon {
    font-size: 16px;
  }

  &__header-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  &__title-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  &__title-text {
    font-weight: 600;
    font-size: 14px;
    color: rgb(var(--color-text));
    letter-spacing: 0.3px;
  }

  &__provider-badge {
    font-size: 10px;
    font-weight: 500;
    padding: 2px 6px;
    background: rgb(var(--color-panel-muted));
    border: 1px solid rgb(var(--color-border));
    color: rgb(var(--color-text-muted));
    border-radius: 4px;
    font-family: monospace;
  }

  &__status {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  &__status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: rgb(var(--color-text-muted));

    &--connected {
      background: #4ade80;
      box-shadow: 0 0 6px rgba(74, 222, 128, 0.6);
    }
  }

  &__status-text {
    font-size: 11px;
    color: rgb(var(--color-text-muted));
  }

  &__actions {
    display: flex;
    gap: 4px;
  }

  &__btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    background: rgb(var(--color-panel-muted));
    color: rgb(var(--color-text-muted));
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s;

    &:hover {
      background: rgb(var(--color-panel));
      color: rgb(var(--color-text));
    }
  }

  &__messages {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 0;

    &::-webkit-scrollbar {
      width: 8px;
    }

    &::-webkit-scrollbar-track {
      background: transparent;
    }

    &::-webkit-scrollbar-thumb {
      background: rgb(var(--color-border));
      border-radius: 4px;

      &:hover {
        background: rgb(var(--color-text-muted));
      }
    }
  }

  &__welcome {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    text-align: center;
    padding: 20px;
    color: rgb(var(--color-text-muted));

    h3 {
      margin: 12px 0 6px;
      font-size: 14px;
      font-weight: 500;
      color: rgb(var(--color-text));
    }

    p {
      margin: 0;
      font-size: 12px;
    }
  }

  &__scroll-anchor {
    height: 1px;
  }

  &__messages-content {
    display: flex;
    flex-direction: column;
    min-height: 100%;
  }
}
</style>
