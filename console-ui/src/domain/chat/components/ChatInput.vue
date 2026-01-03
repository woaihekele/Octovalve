<template>
  <div class="chat-input">
    <div class="chat-input__container" :class="{ 'chat-input__container--focused': isFocused }">
      <!-- Text area (native, borderless) -->
      <textarea
        ref="textareaRef"
        v-model="inputValue"
        :placeholder="placeholder"
        :disabled="disabled"
        class="chat-input__textarea"
        rows="2"
        @keydown="handleKeyDown"
        @compositionstart="handleCompositionStart"
        @compositionend="handleCompositionEnd"
        @focus="isFocused = true"
        @blur="isFocused = false"
        @input="autoResize"
      />
      
      <!-- Toolbar inside container -->
      <div class="chat-input__toolbar">
        <div class="chat-input__toolbar-left">
          <!-- Provider selector -->
          <n-select
            :value="provider"
            :options="providerOptions"
            size="tiny"
            :consistent-menu-width="false"
            class="chat-input__provider-select"
            @update:value="$emit('change-provider', $event)"
          />
        </div>
        <div class="chat-input__toolbar-right">
          <!-- Send / Stop button -->
          <n-button
            v-if="isStreaming"
            size="small"
            type="error"
            circle
            @click="$emit('cancel')"
          >
            <template #icon>
              <n-icon :component="StopOutline" />
            </template>
          </n-button>
          <n-button
            v-else
            size="small"
            type="primary"
            circle
            :disabled="!canSend"
            @click="handleSend"
          >
            <template #icon>
              <n-icon :component="SendOutline" />
            </template>
          </n-button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, nextTick, onMounted } from 'vue';
import { NButton, NIcon, NSelect } from 'naive-ui';
import { SendOutline, StopOutline } from '@vicons/ionicons5';

interface Props {
  modelValue: string;
  placeholder?: string;
  disabled?: boolean;
  isStreaming?: boolean;
  provider?: 'acp' | 'openai';
}

const props = withDefaults(defineProps<Props>(), {
  placeholder: '输入消息...',
  disabled: false,
  isStreaming: false,
  provider: 'acp',
});

const emit = defineEmits<{
  'update:modelValue': [value: string];
  send: [content: string];
  cancel: [];
  'change-provider': [provider: 'acp' | 'openai'];
}>();

const textareaRef = ref<HTMLTextAreaElement | null>(null);
const isFocused = ref(false);
const isComposing = ref(false);
let compositionEndTimer: ReturnType<typeof setTimeout> | null = null;

function handleCompositionStart() {
  isComposing.value = true;
  if (compositionEndTimer) {
    clearTimeout(compositionEndTimer);
    compositionEndTimer = null;
  }
}

function handleCompositionEnd() {
  // Delay setting isComposing to false to allow the Enter key event to be processed
  compositionEndTimer = setTimeout(() => {
    isComposing.value = false;
  }, 100);
}

const providerOptions = [
  { label: 'Codex CLI (ACP)', value: 'acp' },
  { label: 'OpenAI API', value: 'openai' },
];

const inputValue = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value),
});

const canSend = computed(() => {
  return !props.disabled && !props.isStreaming && inputValue.value.trim().length > 0;
});

function handleKeyDown(event: KeyboardEvent) {
  // Shift+Enter = newline, Enter = send (but not during IME composition)
  // Check both our state and the native isComposing property
  if (event.key === 'Enter' && !event.shiftKey && !isComposing.value && !event.isComposing) {
    event.preventDefault();
    handleSend();
  }
}

function handleSend() {
  if (!canSend.value) return;
  emit('send', inputValue.value.trim());
  inputValue.value = '';
  nextTick(autoResize);
}

function autoResize() {
  if (!textareaRef.value) return;
  textareaRef.value.style.height = 'auto';
  const maxHeight = 200;
  textareaRef.value.style.height = `${Math.min(textareaRef.value.scrollHeight, maxHeight)}px`;
}

function focus() {
  nextTick(() => {
    textareaRef.value?.focus();
  });
}

onMounted(() => {
  autoResize();
});

defineExpose({ focus });
</script>

<style scoped lang="scss">
.chat-input {
  padding: 12px 14px;
  background: rgb(var(--color-panel));
  border-top: 1px solid rgb(var(--color-border));

  &__container {
    background: rgb(var(--color-panel-muted));
    border: 1px solid rgb(var(--color-border));
    border-radius: 16px;
    overflow: hidden;
    transition: all 0.2s;

    &--focused {
      border-color: rgb(var(--color-accent));
      box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.18);
      background: rgb(var(--color-panel));
    }
  }

  &__textarea {
    width: 100%;
    border: none;
    outline: none;
    background: transparent;
    padding: 12px 14px 8px;
    font-size: 14px;
    line-height: 1.5;
    resize: none;
    font-family: inherit;
    color: rgb(var(--color-text));
    min-height: 50px;
    max-height: 200px;

    &::placeholder {
      color: rgb(var(--color-text-muted));
    }

    &:disabled {
      opacity: 0.5;
      cursor: not-allowed;
    }
  }

  &__toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px 10px;
    gap: 8px;
  }

  &__toolbar-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  &__toolbar-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  &__provider-select {
    width: 140px;
    
    :deep(.n-base-selection) {
      --n-height: 26px;
      --n-font-size: 12px;
      --n-border: 1px solid rgb(var(--color-border));
      --n-border-hover: 1px solid rgb(var(--color-border));
      --n-border-active: 1px solid rgb(var(--color-accent));
      --n-border-focus: 1px solid rgb(var(--color-accent));
      --n-box-shadow-focus: 0 0 0 2px rgba(99, 102, 241, 0.18);
      border-radius: 6px;
    }
  }
}
</style>
