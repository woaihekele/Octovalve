<template>
  <slot :html="renderedHtml">
    <div class="chat-markdown" v-html="renderedHtml"></div>
  </slot>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useSmoothStream } from '../composables/useSmoothStream';
import { renderSafeMarkdown } from '../utils/markdown';

type SmoothOptions = {
  minDelay?: number;
  chunkFactor?: number;
  maxCharsPerFrame?: number;
  streamEndStrategy?: 'immediate' | 'progressive';
};

interface Props {
  text: string;
  streaming: boolean;
  contentKey?: string;
  smoothOptions?: SmoothOptions;
  prepareStreamingContent?: (text: string, options: { streaming: boolean }) => string;
}

const props = defineProps<Props>();

const displayedText = ref('');
const streamDone = ref(!props.streaming);
const previousRawText = ref('');
const previousKey = ref(props.contentKey ?? '');

const smoothOptions = computed<SmoothOptions>(() => props.smoothOptions || {});

const { addChunk, reset } = useSmoothStream({
  onUpdate: (text) => {
    displayedText.value = text;
  },
  streamDone: () => streamDone.value,
  initialText: '',
  minDelay: smoothOptions.value.minDelay,
  chunkFactor: smoothOptions.value.chunkFactor,
  maxCharsPerFrame: smoothOptions.value.maxCharsPerFrame,
  streamEndStrategy: smoothOptions.value.streamEndStrategy,
});

const prepareText = (text: string, streaming: boolean): string => {
  const normalized = text ?? '';
  return props.prepareStreamingContent
    ? props.prepareStreamingContent(normalized, { streaming })
    : normalized;
};

const handleReset = (raw: string) => {
  reset(raw);
  previousRawText.value = raw;
};

watch(
  () => ({
    text: props.text,
    streaming: props.streaming,
    key: props.contentKey ?? '',
  }),
  ({ text, streaming, key }) => {
    const raw = text || '';
    const prev = previousRawText.value || '';
    const keyChanged = key !== previousKey.value;
    const contentReset = Boolean(prev) && Boolean(raw) && !raw.startsWith(prev);

    if (!prev && raw && !streaming) {
      handleReset(raw);
    } else if (keyChanged || contentReset) {
      handleReset(raw);
    } else {
      const delta = raw.slice(prev.length);
      if (delta) {
        addChunk(delta);
        previousRawText.value = raw;
      }
      if (!delta && !streaming && raw !== prev) {
        handleReset(raw);
      }
    }

    if (!raw && prev) {
      handleReset('');
    }

    previousKey.value = key;
    streamDone.value = !streaming;
  },
  { immediate: true }
);

const renderedSource = computed(() => prepareText(displayedText.value || '', !streamDone.value));
const renderedHtml = computed(() => renderSafeMarkdown(renderedSource.value));
</script>

<style scoped>
.chat-markdown {
  width: 100%;
}
</style>
