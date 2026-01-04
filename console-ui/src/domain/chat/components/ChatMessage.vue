<template>
  <div class="chat-message" :class="[`chat-message--${message.role}`, `chat-message--${message.status}`]">
    <div class="chat-message__avatar">
      <n-icon v-if="message.role === 'user'" :component="PersonOutline" :size="20" />
      <n-icon v-else :component="SparklesOutline" :size="20" />
    </div>
    <div class="chat-message__content">
      <div class="chat-message__header">
        <span class="chat-message__role">{{ roleLabel }}</span>
        <span class="chat-message__time">{{ formattedTime }}</span>
        <n-tag v-if="message.status === 'streaming'" type="info" size="small" round>
          <template #icon>
            <n-spin size="small" />
          </template>
          {{ $t('chat.status.streaming') }}
        </n-tag>
        <n-tag v-else-if="message.status === 'error'" type="error" size="small" round>
          {{ $t('chat.status.error') }}
        </n-tag>
      </div>
      <div class="chat-message__body">
        <div v-if="message.reasoning" class="chat-message__reasoning">
          <n-collapse>
            <n-collapse-item :title="$t('chat.reasoning')" name="reasoning">
              <div class="chat-message__reasoning-content">{{ message.reasoning }}</div>
            </n-collapse-item>
          </n-collapse>
        </div>
        <div class="chat-message__text" v-html="renderedContent"></div>
        <div v-if="message.images?.length" class="chat-message__images">
          <img
            v-for="(img, idx) in message.images"
            :key="idx"
            :src="img"
            class="chat-message__image"
            @click="$emit('image-click', img)"
          />
        </div>
        <div v-if="message.toolCalls?.length" class="chat-message__tools">
          <div v-for="tool in message.toolCalls" :key="tool.id" class="chat-message__tool">
            <n-tag :type="toolStatusType(tool.status)" size="small">
              {{ tool.name }}
            </n-tag>
            <span v-if="tool.result" class="chat-message__tool-result">{{ tool.result }}</span>
          </div>
        </div>
      </div>
      <div v-if="showActions" class="chat-message__actions">
        <n-button size="small" type="primary" @click="$emit('approve')">
          {{ primaryLabel }}
        </n-button>
        <n-button size="small" @click="$emit('reject')">
          {{ secondaryLabel }}
        </n-button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import { NIcon, NTag, NSpin, NButton, NCollapse, NCollapseItem } from 'naive-ui';
import { PersonOutline, SparklesOutline } from '@vicons/ionicons5';
import { useI18n } from 'vue-i18n';
import type { ChatMessage, ToolCall } from '../types';

interface Props {
  message: ChatMessage;
  showActions?: boolean;
  primaryButtonText?: string;
  secondaryButtonText?: string;
}

const props = withDefaults(defineProps<Props>(), {
  showActions: false,
});

defineEmits<{
  approve: [];
  reject: [];
  'image-click': [url: string];
}>();

const { t, locale } = useI18n();

const roleLabel = computed(() => {
  switch (props.message.role) {
    case 'user':
      return t('chat.role.user');
    case 'assistant':
      return t('chat.role.assistant');
    case 'system':
      return t('chat.role.system');
    default:
      return props.message.role;
  }
});

const formattedTime = computed(() => {
  const date = new Date(props.message.ts);
  return date.toLocaleTimeString(locale.value, { hour: '2-digit', minute: '2-digit' });
});

const primaryLabel = computed(() => props.primaryButtonText ?? t('chat.action.approve'));
const secondaryLabel = computed(() => props.secondaryButtonText ?? t('chat.action.reject'));

const renderedContent = computed(() => {
  // Simple markdown-like rendering
  let content = props.message.content;
  // Escape HTML
  content = content.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  // Code blocks
  content = content.replace(/```(\w*)\n([\s\S]*?)```/g, '<pre><code class="language-$1">$2</code></pre>');
  // Inline code
  content = content.replace(/`([^`]+)`/g, '<code>$1</code>');
  // Bold
  content = content.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  // Italic
  content = content.replace(/\*([^*]+)\*/g, '<em>$1</em>');
  // Line breaks
  content = content.replace(/\n/g, '<br>');
  return content;
});

function toolStatusType(status: ToolCall['status']): 'default' | 'info' | 'success' | 'error' {
  switch (status) {
    case 'pending':
      return 'default';
    case 'running':
      return 'info';
    case 'completed':
      return 'success';
    case 'failed':
    case 'cancelled':
      return 'error';
    default:
      return 'default';
  }
}
</script>

<style scoped lang="scss">
.chat-message {
  display: flex;
  gap: 12px;
  padding: 12px 16px;
  border-radius: 8px;
  margin-bottom: 8px;

  &--user {
    background: var(--color-panel-muted);
  }

  &--assistant {
    background: var(--color-panel);
  }

  &--error {
    border-left: 3px solid var(--color-danger);
  }

  &__avatar {
    flex-shrink: 0;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: var(--color-accent);
    display: flex;
    align-items: center;
    justify-content: center;
    color: white;
  }

  &--user &__avatar {
    background: var(--color-text-muted);
  }

  &__content {
    flex: 1;
    min-width: 0;
  }

  &__header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
    font-size: 12px;
  }

  &__role {
    font-weight: 600;
    color: var(--color-text);
  }

  &__time {
    color: var(--color-text-muted);
  }

  &__body {
    color: var(--color-text);
    line-height: 1.6;
  }

  &__text {
    word-break: break-word;

    :deep(code) {
      background: var(--color-panel-muted);
      padding: 2px 6px;
      border-radius: 4px;
      font-family: monospace;
      font-size: 0.9em;
    }

    :deep(pre) {
      background: var(--color-panel-muted);
      padding: 12px;
      border-radius: 6px;
      overflow-x: auto;
      margin: 8px 0;

      code {
        background: none;
        padding: 0;
      }
    }
  }

  &__reasoning {
    margin-bottom: 8px;
    font-size: 0.9em;
    opacity: 0.8;
  }

  &__reasoning-content {
    white-space: pre-wrap;
    font-style: italic;
  }

  &__images {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 8px;
  }

  &__image {
    max-width: 200px;
    max-height: 150px;
    border-radius: 6px;
    cursor: pointer;
    transition: transform 0.2s;

    &:hover {
      transform: scale(1.02);
    }
  }

  &__tools {
    margin-top: 8px;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  &__tool {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  &__tool-result {
    font-size: 0.85em;
    color: var(--color-text-muted);
  }

  &__actions {
    display: flex;
    gap: 8px;
    margin-top: 12px;
  }
}

@keyframes blink {
  0%,
  50% {
    opacity: 1;
  }
  51%,
  100% {
    opacity: 0;
  }
}
</style>
