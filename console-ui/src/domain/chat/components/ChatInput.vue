<template>
  <div class="chat-input">
    <div
      class="chat-input__container"
      :class="{
        'chat-input__container--focused': isFocused,
        'chat-input__container--mention': mentionOpen,
      }"
      @dragover.stop.prevent="handleDragOver"
      @drop.stop.prevent="handleDrop"
    >
      <input
        ref="fileInputRef"
        type="file"
        :accept="fileAccept"
        multiple
        class="chat-input__file-input"
        @change="handleFileSelect"
      />
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
        @paste="handlePaste"
        @dragover.stop.prevent="handleDragOver"
        @drop.stop.prevent="handleDrop"
      />

      <div v-if="attachments.length > 0" class="chat-input__attachments">
        <div
          v-for="(attachment, index) in attachments"
          :key="`${attachment.kind}-${attachment.name ?? index}`"
          class="chat-input__attachment"
          :class="{ 'chat-input__attachment--file': attachment.kind === 'text' }"
        >
          <img
            v-if="attachment.kind === 'image'"
            :src="attachment.previewUrl"
            :alt="attachment.name || 'image'"
            class="chat-input__attachment-thumb"
          />
          <div v-else class="chat-input__attachment-file">
            <span class="chat-input__attachment-file-label">{{ attachment.name }}</span>
          </div>
          <button type="button" class="chat-input__attachment-remove" @click="removeAttachment(index)">
            ×
          </button>
        </div>
      </div>

      <div v-if="attachmentError" class="chat-input__attachment-error">
        {{ attachmentError }}
      </div>

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
          <n-button
            size="tiny"
            text
            class="chat-input__image-button"
            :disabled="disabled || isStreaming"
            @click="triggerFileSelect"
          >
            <template #icon>
              <n-icon :component="AddOutline" />
            </template>
          </n-button>
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
import { AddOutline, StopOutline } from '@vicons/ionicons5';
import { useI18n } from 'vue-i18n';
import type { TargetInfo } from '../../../shared/types';
import type { ImageAttachment, SendMessageOptions, TextAttachment } from '../types';

interface Props {
  modelValue: string;
  placeholder?: string;
  disabled?: boolean;
  isStreaming?: boolean;
  provider?: 'acp' | 'openai';
  sendOnEnter?: boolean;
  targets?: TargetInfo[];
  supportsImage?: boolean;
}

const props = withDefaults(defineProps<Props>(), {
  disabled: false,
  isStreaming: false,
  provider: 'acp',
  sendOnEnter: false,
  targets: () => [],
  supportsImage: false,
});

const emit = defineEmits<{
  'update:modelValue': [value: string];
  send: [options: SendMessageOptions];
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
type ChatAttachment = ImageAttachment | TextAttachment;
const attachments = ref<ChatAttachment[]>([]);
const attachmentError = ref('');
const fileInputRef = ref<HTMLInputElement | null>(null);

const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
const MAX_TEXT_BYTES = 512 * 1024;
const MAX_ATTACHMENT_COUNT = 3;

const resolvedPlaceholder = computed(() => props.placeholder ?? t('chat.input.placeholder.default'));
const mentionEmptyLabel = computed(() =>
  props.targets.length === 0 ? t('chat.input.mention.noTargets') : t('chat.input.mention.noMatch')
);

const fileAccept = computed(() => {
  const parts = ['.txt', '.md', 'text/plain', 'text/markdown'];
  if (props.supportsImage) {
    parts.unshift('image/*');
  }
  return parts.join(',');
});

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
  return (
    !props.disabled &&
    !props.isStreaming &&
    (inputValue.value.trim().length > 0 || attachments.value.length > 0)
  );
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

watch(
  () => props.supportsImage,
  (supports) => {
    if (!supports) {
      const remaining = attachments.value.filter((attachment) => attachment.kind === 'text');
      if (remaining.length !== attachments.value.length) {
        attachments.value = remaining;
        attachmentError.value = '';
      }
    }
  }
);

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
  const imageAttachments = attachments.value.filter(
    (attachment): attachment is ImageAttachment => attachment.kind === 'image'
  );
  const textAttachments = attachments.value.filter(
    (attachment): attachment is TextAttachment => attachment.kind === 'text'
  );
  const options: SendMessageOptions = {
    content: inputValue.value.trim(),
    images: imageAttachments,
    files: textAttachments,
  };
  emit('send', options);
  inputValue.value = '';
  attachments.value = [];
  attachmentError.value = '';
  if (fileInputRef.value) {
    fileInputRef.value.value = '';
  }
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

function triggerFileSelect() {
  if (props.disabled || props.isStreaming) {
    return;
  }
  fileInputRef.value?.click();
}

function handleFileSelect(event: Event) {
  const input = event.target as HTMLInputElement | null;
  const files = input?.files;
  if (!files || files.length === 0) {
    return;
  }
  void addFiles(Array.from(files));
}

function handlePaste(event: ClipboardEvent) {
  if (!props.supportsImage || !event.clipboardData) {
    return;
  }
  const files = Array.from(event.clipboardData.files || []);
  if (files.length === 0) {
    return;
  }
  event.preventDefault();
  void addFiles(files);
}

function handleDragOver(event: DragEvent) {
  if (props.disabled || props.isStreaming) {
    return;
  }
  event.dataTransfer?.setData('text/plain', '');
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = 'copy';
  }
}

function handleDrop(event: DragEvent) {
  if (props.disabled || props.isStreaming) {
    return;
  }
  const files = Array.from(event.dataTransfer?.files || []);
  if (files.length === 0) {
    return;
  }
  void addFiles(files);
}

function removeAttachment(index: number) {
  attachments.value.splice(index, 1);
}

function canAcceptAttachments() {
  return !props.disabled && !props.isStreaming;
}

async function addExternalFiles(files: File[]) {
  if (!canAcceptAttachments()) {
    return;
  }
  await addFiles(files);
}

async function addFiles(files: File[]) {
  attachmentError.value = '';
  const remainingSlots = MAX_ATTACHMENT_COUNT - attachments.value.length;
  if (remainingSlots <= 0) {
    attachmentError.value = `Max ${MAX_ATTACHMENT_COUNT} attachments`;
    return;
  }

  const candidates: File[] = [];
  for (const file of files) {
    if (candidates.length >= remainingSlots) {
      break;
    }
    if (isImageFile(file)) {
      if (props.supportsImage) {
        candidates.push(file);
      }
      continue;
    }
    if (isTextFile(file)) {
      candidates.push(file);
    }
  }

  if (candidates.length === 0) {
    attachmentError.value = props.supportsImage
      ? 'Only image or text files are supported'
      : 'Only text files are supported';
    return;
  }

  for (const file of candidates) {
    if (isImageFile(file)) {
      if (file.size > MAX_IMAGE_BYTES) {
        attachmentError.value = `Image must be smaller than ${Math.round(MAX_IMAGE_BYTES / 1024 / 1024)}MB`;
        continue;
      }
      try {
        const dataUrl = await readFileAsDataUrl(file);
        const parsed = parseDataUrl(dataUrl);
        if (!parsed) {
          attachmentError.value = 'Failed to read image';
          continue;
        }
        attachments.value.push({
          kind: 'image',
          data: parsed.data,
          mimeType: parsed.mimeType,
          previewUrl: dataUrl,
          name: file.name,
          size: file.size,
        });
      } catch (err) {
        console.warn('Failed to read image:', err);
        attachmentError.value = 'Failed to read image';
      }
      continue;
    }

    if (file.size > MAX_TEXT_BYTES) {
      attachmentError.value = `Text file must be smaller than ${Math.round(MAX_TEXT_BYTES / 1024)}KB`;
      continue;
    }
    try {
      const text = await readFileAsText(file);
      attachments.value.push({
        kind: 'text',
        name: file.name || 'file',
        mimeType: file.type || inferTextMimeType(file.name),
        content: text,
        size: file.size,
      });
    } catch (err) {
      console.warn('Failed to read text file:', err);
      attachmentError.value = 'Failed to read text file';
    }
  }
}

function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result || ''));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

function parseDataUrl(value: string): { mimeType: string; data: string } | null {
  const match = value.match(/^data:(.*?);base64,(.*)$/);
  if (!match) {
    return null;
  }
  return { mimeType: match[1], data: match[2] };
}

function readFileAsText(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result || ''));
    reader.onerror = () => reject(reader.error);
    reader.readAsText(file);
  });
}

function isImageFile(file: File): boolean {
  if (file.type.startsWith('image/')) {
    return true;
  }
  const name = file.name.toLowerCase();
  return (
    name.endsWith('.png') ||
    name.endsWith('.jpg') ||
    name.endsWith('.jpeg') ||
    name.endsWith('.webp')
  );
}

function isTextFile(file: File): boolean {
  if (file.type.startsWith('text/')) {
    return true;
  }
  const name = file.name.toLowerCase();
  return name.endsWith('.txt') || name.endsWith('.md');
}

function inferTextMimeType(name: string): string {
  const lower = name.toLowerCase();
  if (lower.endsWith('.md')) {
    return 'text/markdown';
  }
  return 'text/plain';
}

function focus() {
  nextTick(() => {
    textareaRef.value?.focus();
  });
}

onMounted(() => {
  autoResize();
});

defineExpose({ focus, addExternalFiles });
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

  &__file-input {
    display: none;
  }

  &__attachments {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 0 12px 8px;
  }

  &__attachment {
    position: relative;
    width: 64px;
    height: 64px;
  }

  &__attachment--file {
    width: auto;
    min-width: 140px;
    height: 56px;
  }

  &__attachment-thumb {
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: 8px;
    border: 1px solid rgb(var(--color-border));
  }

  &__attachment-file {
    display: flex;
    align-items: center;
    height: 100%;
    padding: 0 12px;
    border-radius: 8px;
    border: 1px solid rgb(var(--color-border));
    background: rgb(var(--color-panel));
    color: rgb(var(--color-text));
    font-size: 12px;
    max-width: 220px;
  }

  &__attachment-file-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  &__attachment-remove {
    position: absolute;
    top: -6px;
    right: -6px;
    width: 18px;
    height: 18px;
    border-radius: 999px;
    border: 1px solid rgb(var(--color-border));
    background: rgb(var(--color-panel));
    color: rgb(var(--color-text));
    font-size: 12px;
    line-height: 1;
    cursor: pointer;
  }

  &__attachment-error {
    padding: 0 12px 6px;
    font-size: 11px;
    color: rgb(var(--color-danger));
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

  &__image-button {
    padding: 0;
    min-width: 0;
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  &__send-icon {
    display: block;
  }

  &__provider-select {
    width: 70px;
    
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
