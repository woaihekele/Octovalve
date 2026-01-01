<template>
  <div class="chat-input">
    <!-- Text area -->
    <div class="chat-input__textarea-wrapper" :class="{ 'chat-input__textarea-wrapper--focused': isFocused }">
      <n-input
        ref="inputRef"
        v-model:value="inputValue"
        type="textarea"
        :autosize="{ minRows: 2, maxRows: 8 }"
        :placeholder="placeholder"
        :disabled="disabled"
        @keydown="handleKeyDown"
        @compositionstart="isComposing = true"
        @compositionend="isComposing = false"
        @focus="isFocused = true"
        @blur="isFocused = false"
      />
    </div>
    
    <!-- Toolbar -->
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
</template>

<script setup lang="ts">
import { ref, computed, nextTick } from 'vue';
import { NInput, NButton, NIcon, NSelect } from 'naive-ui';
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

const inputRef = ref<InstanceType<typeof NInput> | null>(null);
const isFocused = ref(false);
const isComposing = ref(false);

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
  if (event.key === 'Enter' && !event.shiftKey && !isComposing.value) {
    event.preventDefault();
    handleSend();
  }
}

function handleSend() {
  if (!canSend.value) return;
  emit('send', inputValue.value.trim());
  inputValue.value = '';
}

function focus() {
  nextTick(() => {
    inputRef.value?.focus();
  });
}

defineExpose({ focus });
</script>

<style scoped lang="scss">
.chat-input {
  padding: 12px 14px;
  background: rgb(var(--color-panel));
  border-top: 1px solid rgb(var(--color-border));

  &__textarea-wrapper {
    background: #f9fafb;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    overflow: hidden;
    transition: all 0.2s;

    &--focused {
      border-color: #8b5cf6;
      box-shadow: 0 0 0 3px rgba(139, 92, 246, 0.1);
      background: white;
    }

    :deep(.n-input) {
      --n-border: none;
      --n-border-hover: none;
      --n-border-focus: none;
      --n-box-shadow-focus: none;
      --n-color: transparent;
      --n-color-focus: transparent;
      
      .n-input-wrapper {
        padding: 10px 12px;
      }

      .n-input__textarea-el {
        resize: none;
        font-size: 14px;
        line-height: 1.5;
      }
    }
  }

  &__toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: 10px;
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
      --n-height: 28px;
      --n-font-size: 12px;
    }
  }
}
</style>
