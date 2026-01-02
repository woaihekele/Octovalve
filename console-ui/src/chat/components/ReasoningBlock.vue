<template>
  <div class="reasoning-block" v-if="hasBody">
    <div class="reasoning-header" @click="$emit('toggle')">
      <span>{{ show ? '收起思考过程' : '展开思考过程' }}</span>
      <svg class="caret" width="12" height="12" viewBox="0 0 24 24">
        <path
          :d="show ? 'M7 14l5-5 5 5' : 'M7 10l5 5 5-5'"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </div>

    <div v-show="show" class="reasoning-body">
      <slot name="body" v-if="$slots.body"></slot>
      <div v-else v-html="bodyHtml"></div>
    </div>

    <div
      v-if="showPreview"
      ref="previewRef"
      class="reasoning-preview"
      @wheel.prevent
      @touchmove.prevent
    >
      <MarkdownRender
        :custom-id="previewCustomId || 'reasoning-preview'"
        :content="displayedPreviewText"
        :is-dark="props.isDark ?? true"
        :max-live-nodes="0"
        :batch-rendering="true"
        :render-batch-size="16"
        :render-batch-delay="8"
        :final="previewStreamDone"
      />
    </div>

    <div v-show="show" class="collapse-footer" @click="$emit('toggle')">
      <svg class="caret" width="12" height="12" viewBox="0 0 24 24">
        <path
          d="M7 14l5-5 5 5"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, useSlots, watch } from 'vue';
import MarkdownRender from 'markstream-vue';
import { useSmoothStream } from '../composables/useSmoothStream';

interface Props {
  show: boolean;
  bodyHtml?: string;
  streaming?: boolean;
  previewText?: string;
  previewActive?: boolean;
  previewCustomId?: string;
  isDark?: boolean;
  smoothOptions?: {
    minDelay?: number;
    chunkFactor?: number;
    maxCharsPerFrame?: number;
    streamEndStrategy?: 'immediate' | 'progressive';
  };
}

const props = defineProps<Props>();
const slots = useSlots();
const previewRef = ref<HTMLElement | null>(null);
const displayedPreviewText = ref('');
const previewStreamDone = ref(!props.streaming);
const previousPreviewRaw = ref('');

let previewScrollFrame: number | null = null;
let previewScrollToken = 0;
let previewScrollTarget = 0;

const stopPreviewFollow = () => {
  previewScrollToken += 1;
  if (previewScrollFrame !== null) {
    cancelAnimationFrame(previewScrollFrame);
    previewScrollFrame = null;
  }
};

const startPreviewFollow = () => {
  const element = previewRef.value;
  if (!element) {
    return;
  }
  previewScrollTarget = Math.max(element.scrollHeight - element.clientHeight, 0);
  if (previewScrollFrame !== null) {
    return;
  }

  const token = previewScrollToken + 1;
  previewScrollToken = token;

  const step = () => {
    if (previewScrollToken !== token) {
      previewScrollFrame = null;
      return;
    }
    const el = previewRef.value;
    if (!el) {
      previewScrollFrame = null;
      return;
    }

    previewScrollTarget = Math.max(el.scrollHeight - el.clientHeight, 0);
    const current = el.scrollTop;
    const delta = previewScrollTarget - current;

    if (Math.abs(delta) <= 0.5) {
      el.scrollTop = previewScrollTarget;
      previewScrollFrame = null;
      return;
    }

    el.scrollTop = current + delta * 0.35;
    previewScrollFrame = requestAnimationFrame(step);
  };

  previewScrollFrame = requestAnimationFrame(step);
};

defineEmits<{ (e: 'toggle'): void }>();

const hasBody = computed(() => Boolean(props.bodyHtml) || Boolean(slots.body));

const normalizePreviewText = (text: string): string => {
  return text || '';
};

const smoothOptions = computed(() => props.smoothOptions || {});

const { addChunk: addPreviewChunk, reset: resetPreviewStream } = useSmoothStream({
  onUpdate: (text) => {
    displayedPreviewText.value = text;
  },
  streamDone: () => previewStreamDone.value,
  initialText: '',
  minDelay: smoothOptions.value.minDelay,
  chunkFactor: smoothOptions.value.chunkFactor,
  maxCharsPerFrame: smoothOptions.value.maxCharsPerFrame,
  streamEndStrategy: smoothOptions.value.streamEndStrategy,
});

watch(
  () => ({
    text: props.previewText || '',
    streaming: Boolean(props.streaming),
  }),
  ({ text, streaming }) => {
    const raw = normalizePreviewText(text);
    const prev = previousPreviewRaw.value || '';
    const contentReset = Boolean(prev) && Boolean(raw) && !raw.startsWith(prev);

    if (!prev && raw && !streaming) {
      resetPreviewStream(raw);
      previousPreviewRaw.value = raw;
    } else if (contentReset) {
      resetPreviewStream(raw);
      previousPreviewRaw.value = raw;
    } else {
      const delta = raw.slice(prev.length);
      if (delta) {
        addPreviewChunk(delta);
        previousPreviewRaw.value = raw;
      }
      if (!delta && !streaming && raw !== prev) {
        resetPreviewStream(raw);
        previousPreviewRaw.value = raw;
      }
    }

    if (!raw && prev) {
      resetPreviewStream('');
      previousPreviewRaw.value = '';
    }

    previewStreamDone.value = !streaming;
  },
  { immediate: true }
);

watch(
  () => props.show,
  (visible) => {
    if (!visible) {
      return;
    }

    const raw = normalizePreviewText(props.previewText || '');
    resetPreviewStream(raw);
    previousPreviewRaw.value = raw;
  }
);

const previewLines = computed(() => {
  const text = displayedPreviewText.value || '';
  if (!text) return [];
  const normalized = text.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
  const lines = normalized.split('\n');
  while (lines.length > 0 && lines[lines.length - 1] === '') {
    lines.pop();
  }
  return lines;
});

const showPreview = computed(() => {
  return !props.show && (props.previewActive ?? true) && previewLines.value.length > 0;
});

watch(
  () => showPreview.value,
  async (visible) => {
    if (!visible) {
      stopPreviewFollow();
      return;
    }
    if (!visible) return;
    await nextTick();
    if (previewRef.value) {
      previewRef.value.scrollTop = previewRef.value.scrollHeight;
    }
  },
  { flush: 'post' }
);

watch(
  () => previewLines.value.length,
  async (count, prev) => {
    if (!showPreview.value || count <= prev) return;
    await nextTick();
    if (previewRef.value) {
      startPreviewFollow();
    }
  },
  { flush: 'post' }
);

onBeforeUnmount(() => {
  stopPreviewFollow();
});
</script>

<style scoped lang="scss">
.reasoning-block {
  margin-bottom: 8px;
  border-radius: 12px;
  overflow: hidden;
  background: rgb(var(--color-panel-muted));
  border: 1px solid rgb(var(--color-border));
}

.reasoning-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 12px;
  cursor: pointer;
  font-size: 12px;
  color: rgb(var(--color-text-muted));
  user-select: none;
}

.caret {
  opacity: 0.9;
}

.reasoning-body {
  padding: 10px 12px;
  color: rgb(var(--color-text));
}

.reasoning-preview {
  padding: 10px 12px;
  max-height: 110px;
  overflow: hidden;
}

.reasoning-line {
  font-size: 12px;
  line-height: 1.4;
  color: rgb(var(--color-text-muted));
  white-space: pre-wrap;
}

.reasoning-line.empty {
  height: 1.4em;
}

.collapse-footer {
  display: flex;
  justify-content: center;
  padding: 6px 12px;
  cursor: pointer;
  color: rgb(var(--color-text-muted));
}
</style>
