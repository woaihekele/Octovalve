<template>
  <div class="chat-input">
    <div
      class="chat-input__container"
      :class="{
        'chat-input__container--focused': isFocused,
        'chat-input__container--mention': mentionOpen,
      }"
    >
      <!-- Text area (native, borderless) -->
      <textarea
        ref="textareaRef"
        v-model="inputValue"
        :placeholder="resolvedPlaceholder"
        :disabled="disabled"
        class="chat-input__textarea"
        rows="2"
        @keydown="handleKeyDown"
        @keyup="handleKeyUp"
        @compositionstart="handleCompositionStart"
        @compositionend="handleCompositionEnd"
        @focus="handleFocus"
        @blur="handleBlur"
        @click="handleClick"
        @input="handleInput"
      />

      <div v-if="mentionOpen" class="chat-input__mention scrollbar-chat">
        <div v-if="filteredTargets.length === 0" class="chat-input__mention-empty">
          {{ mentionEmptyLabel }}
        </div>
        <button
          v-for="(name, index) in filteredTargets"
          :key="name"
          type="button"
          class="chat-input__mention-item"
          :class="{ 'chat-input__mention-item--active': index === activeIndex }"
          @mousedown.prevent="selectMention(name)"
        >
          @{{ name }}
        </button>
      </div>

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
            text
            class="chat-input__send-button"
            :disabled="!canSend"
            @click="handleSend"
          >
            <template #icon>
              <svg
                class="h-4 w-4 chat-input__send-icon"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.6"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M22 2L11 13" />
                <path d="M22 2L15 22L11 13L2 9L22 2Z" />
              </svg>
            </template>
          </n-button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, nextTick, onMounted, watch } from 'vue';
import { NButton, NIcon, NSelect } from 'naive-ui';
import { StopOutline } from '@vicons/ionicons5';
import { useI18n } from 'vue-i18n';
import type { TargetInfo } from '../../../shared/types';

interface Props {
  modelValue: string;
  placeholder?: string;
  disabled?: boolean;
  isStreaming?: boolean;
  provider?: 'acp' | 'openai';
  sendOnEnter?: boolean;
  targets?: TargetInfo[];
}

const props = withDefaults(defineProps<Props>(), {
  disabled: false,
  isStreaming: false,
  provider: 'acp',
  sendOnEnter: false,
  targets: () => [],
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
const ignoreNextEnter = ref(false);
const { t } = useI18n();
const mentionOpen = ref(false);
const mentionQuery = ref('');
const mentionRangeStart = ref(-1);
const mentionRangeEnd = ref(-1);
const activeIndex = ref(0);

const resolvedPlaceholder = computed(() => props.placeholder ?? t('chat.input.placeholder.default'));
const mentionEmptyLabel = computed(() =>
  props.targets.length === 0 ? t('chat.input.mention.noTargets') : t('chat.input.mention.noMatch')
);

function handleCompositionStart() {
  isComposing.value = true;
  ignoreNextEnter.value = false;
}

function handleCompositionEnd() {
  isComposing.value = false;
  if (props.sendOnEnter) {
    ignoreNextEnter.value = true;
  }
  nextTick(updateMentionState);
}

const providerOptions = computed(() => [
  { label: t('chat.input.provider.acp'), value: 'acp' },
  { label: t('chat.input.provider.openai'), value: 'openai' },
]);

const inputValue = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value),
});

const canSend = computed(() => {
  return !props.disabled && !props.isStreaming && inputValue.value.trim().length > 0;
});

const targetNames = computed(() => {
  const names = props.targets.map((target) => target.name);
  return Array.from(new Set(names));
});

function fuzzyScore(target: string, query: string) {
  if (!query) return 0;
  let score = 0;
  let cursor = 0;
  for (const char of query) {
    const found = target.indexOf(char, cursor);
    if (found === -1) return null;
    score += found - cursor;
    cursor = found + 1;
  }
  return score + Math.max(0, target.length - query.length);
}

const filteredTargets = computed(() => {
  if (!mentionOpen.value) {
    return [];
  }
  const query = mentionQuery.value.trim().toLowerCase();
  const candidates = targetNames.value.map((name) => name.trim()).filter(Boolean);
  if (!query) {
    return candidates;
  }
  return candidates
    .map((name) => {
      const score = fuzzyScore(name.toLowerCase(), query);
      if (score === null) {
        return null;
      }
      return { name, score };
    })
    .filter((item): item is { name: string; score: number } => item !== null)
    .sort((a, b) => a.score - b.score || a.name.localeCompare(b.name))
    .map((item) => item.name);
});

watch(filteredTargets, (list) => {
  if (activeIndex.value >= list.length) {
    activeIndex.value = 0;
  }
});

function closeMention() {
  mentionOpen.value = false;
  mentionQuery.value = '';
  mentionRangeStart.value = -1;
  mentionRangeEnd.value = -1;
  activeIndex.value = 0;
}

function setMentionState(start: number, end: number, query: string) {
  const shouldReset =
    !mentionOpen.value ||
    mentionRangeStart.value !== start ||
    mentionRangeEnd.value !== end ||
    mentionQuery.value !== query;
  mentionOpen.value = true;
  mentionRangeStart.value = start;
  mentionRangeEnd.value = end;
  mentionQuery.value = query;
  if (shouldReset) {
    activeIndex.value = 0;
  }
}

function updateMentionState() {
  if (isComposing.value) {
    return;
  }
  const textarea = textareaRef.value;
  if (!textarea) {
    closeMention();
    return;
  }
  const caret = textarea.selectionStart ?? inputValue.value.length;
  const before = inputValue.value.slice(0, caret);
  const atIndex = before.lastIndexOf('@');
  if (atIndex === -1) {
    closeMention();
    return;
  }
  if (atIndex > 0 && !/\s/.test(before[atIndex - 1])) {
    closeMention();
    return;
  }
  const afterAt = inputValue.value.slice(atIndex + 1);
  const nextSpace = afterAt.search(/\s/);
  const mentionEnd = nextSpace === -1 ? inputValue.value.length : atIndex + 1 + nextSpace;
  if (caret > mentionEnd) {
    closeMention();
    return;
  }
  const query = inputValue.value.slice(atIndex + 1, caret);
  setMentionState(atIndex, mentionEnd, query);
}

function selectMention(name: string) {
  if (!mentionOpen.value) {
    return;
  }
  const start = mentionRangeStart.value;
  const end = mentionRangeEnd.value;
  if (start < 0 || end < start) {
    closeMention();
    return;
  }
  const before = inputValue.value.slice(0, start);
  const after = inputValue.value.slice(end);
  const insert = `@${name} `;
  inputValue.value = `${before}${insert}${after}`;
  closeMention();
  nextTick(() => {
    if (!textareaRef.value) {
      return;
    }
    const position = before.length + insert.length;
    textareaRef.value.setSelectionRange(position, position);
    textareaRef.value.focus();
    autoResize();
  });
}

function handleKeyDown(event: KeyboardEvent) {
  if (mentionOpen.value) {
    if (event.key === 'Tab') {
      event.preventDefault();
      if (filteredTargets.value.length > 0) {
        const name = filteredTargets.value[activeIndex.value] ?? filteredTargets.value[0];
        if (name) {
          selectMention(name);
        }
      } else {
        closeMention();
      }
      return;
    }
    if (event.key === 'Enter') {
      if (isComposing.value || event.isComposing) {
        return;
      }
      event.preventDefault();
      if (filteredTargets.value.length > 0) {
        const name = filteredTargets.value[activeIndex.value] ?? filteredTargets.value[0];
        if (name) {
          selectMention(name);
        }
      } else {
        closeMention();
      }
      return;
    }
    if (event.key === 'ArrowDown') {
      if (filteredTargets.value.length > 0) {
        event.preventDefault();
        activeIndex.value = (activeIndex.value + 1) % filteredTargets.value.length;
      }
      return;
    }
    if (event.key === 'ArrowUp') {
      if (filteredTargets.value.length > 0) {
        event.preventDefault();
        activeIndex.value =
          (activeIndex.value - 1 + filteredTargets.value.length) % filteredTargets.value.length;
      }
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      closeMention();
      return;
    }
  }
  if (event.key !== 'Enter') return;
  if (props.sendOnEnter) {
    if (event.shiftKey || isComposing.value || event.isComposing) {
      return;
    }
    if (ignoreNextEnter.value) {
      ignoreNextEnter.value = false;
      event.preventDefault();
      return;
    }
    event.preventDefault();
    handleSend();
    return;
  }
  if (event.metaKey && !event.shiftKey && !isComposing.value && !event.isComposing) {
    event.preventDefault();
    handleSend();
  }
}

function handleKeyUp(event: KeyboardEvent) {
  if (event.key === 'Tab' || event.key === 'Escape' || event.key === 'Enter') {
    return;
  }
  updateMentionState();
}

function handleSend() {
  if (!canSend.value) return;
  emit('send', inputValue.value.trim());
  inputValue.value = '';
  closeMention();
  nextTick(autoResize);
}

function autoResize() {
  if (!textareaRef.value) return;
  textareaRef.value.style.height = 'auto';
  const maxHeight = 200;
  textareaRef.value.style.height = `${Math.min(textareaRef.value.scrollHeight, maxHeight)}px`;
}

function handleInput() {
  autoResize();
  updateMentionState();
}

function handleClick() {
  updateMentionState();
}

function handleFocus() {
  isFocused.value = true;
  updateMentionState();
}

function handleBlur() {
  isFocused.value = false;
  closeMention();
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
    position: relative;
    transition: all 0.2s;

    &--focused {
      border-color: rgb(var(--color-accent));
      box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.18);
      background: rgb(var(--color-panel));
    }

    &--mention {
      overflow: visible;
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

  &__mention {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + 6px);
    z-index: 20;
    border: 1px solid rgb(var(--color-border));
    border-radius: 10px;
    background: rgb(var(--color-panel));
    box-shadow: 0 12px 24px rgba(0, 0, 0, 0.18);
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 240px;
    overflow-y: auto;
  }

  &__mention-item {
    text-align: left;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 6px 10px;
    font-size: 13px;
    color: rgb(var(--color-text));
    background: transparent;
    cursor: pointer;

    &:hover,
    &--active {
      background: rgb(var(--color-panel-muted));
      border-color: rgb(var(--color-border));
    }
  }

  &__mention-empty {
    font-size: 12px;
    color: rgb(var(--color-text-muted));
    padding: 6px 8px;
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

  &__send-button {
    padding: 0;
    min-width: 0;
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  &__send-icon {
    display: block;
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
