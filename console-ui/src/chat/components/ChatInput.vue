<template>
  <div class="chat-input" :class="{ 'chat-input--focused': isFocused }">
    <div v-if="selectedImages.length || selectedFiles.length" class="chat-input__attachments">
      <n-tag
        v-for="(img, idx) in selectedImages"
        :key="`img-${idx}`"
        closable
        size="small"
        @close="$emit('remove-image', img)"
      >
        <template #icon>
          <n-icon :component="ImageOutline" />
        </template>
        图片 {{ idx + 1 }}
      </n-tag>
      <n-tag
        v-for="(file, idx) in selectedFiles"
        :key="`file-${idx}`"
        closable
        size="small"
        @close="$emit('remove-file', file)"
      >
        <template #icon>
          <n-icon :component="DocumentOutline" />
        </template>
        {{ getFileName(file) }}
      </n-tag>
    </div>
    <div class="chat-input__row">
      <n-input
        ref="inputRef"
        v-model:value="inputValue"
        type="textarea"
        :autosize="{ minRows: 1, maxRows: 6 }"
        :placeholder="placeholder"
        :disabled="disabled"
        @keydown="handleKeyDown"
        @focus="isFocused = true"
        @blur="isFocused = false"
      />
      <div class="chat-input__actions">
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
import { NInput, NButton, NIcon, NTag } from 'naive-ui';
import { SendOutline, StopOutline, ImageOutline, DocumentOutline } from '@vicons/ionicons5';

interface Props {
  modelValue: string;
  placeholder?: string;
  disabled?: boolean;
  isStreaming?: boolean;
  selectedImages?: string[];
  selectedFiles?: string[];
}

const props = withDefaults(defineProps<Props>(), {
  placeholder: '输入消息...',
  disabled: false,
  isStreaming: false,
  selectedImages: () => [],
  selectedFiles: () => [],
});

const emit = defineEmits<{
  'update:modelValue': [value: string];
  send: [content: string];
  cancel: [];
  'remove-image': [url: string];
  'remove-file': [path: string];
}>();

const inputRef = ref<InstanceType<typeof NInput> | null>(null);
const isFocused = ref(false);

const inputValue = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value),
});

const canSend = computed(() => {
  return !props.disabled && !props.isStreaming && inputValue.value.trim().length > 0;
});

function handleKeyDown(event: KeyboardEvent) {
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault();
    handleSend();
  }
}

function handleSend() {
  if (!canSend.value) return;
  emit('send', inputValue.value.trim());
  inputValue.value = '';
}

function getFileName(path: string): string {
  return path.split('/').pop() || path;
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
  padding: 12px 16px;
  background: var(--color-panel);
  border-top: 1px solid var(--color-border);
  border-radius: 0 0 8px 8px;

  &--focused {
    border-top-color: var(--color-accent);
  }

  &__attachments {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 8px;
  }

  &__row {
    display: flex;
    gap: 8px;
    align-items: flex-end;
  }

  &__actions {
    flex-shrink: 0;
    display: flex;
    gap: 4px;
  }

  :deep(.n-input) {
    flex: 1;

    .n-input__textarea-el {
      resize: none;
    }
  }
}
</style>
