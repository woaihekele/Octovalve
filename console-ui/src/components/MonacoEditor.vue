<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import type * as Monaco from 'monaco-editor';

const props = defineProps<{
  modelValue: string;
  language?: string;
  readOnly?: boolean;
  height?: string;
}>();

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void;
}>();

const containerRef = ref<HTMLDivElement | null>(null);
const editorHeight = computed(() => props.height ?? '260px');

let editor: Monaco.editor.IStandaloneCodeEditor | null = null;
let monacoApi: typeof Monaco | null = null;
let updatingFromEditor = false;

async function initEditor() {
  if (!containerRef.value || editor) {
    return;
  }
  monacoApi = await import('monaco-editor');
  await import('monaco-editor/esm/vs/basic-languages/toml/toml');
  monacoApi.editor.setTheme('vs-dark');
  editor = monacoApi.editor.create(containerRef.value, {
    value: props.modelValue ?? '',
    language: props.language ?? 'toml',
    readOnly: props.readOnly ?? false,
    automaticLayout: true,
    minimap: { enabled: false },
    fontSize: 12,
    scrollBeyondLastLine: false,
    tabSize: 2,
  });
  editor.onDidChangeModelContent(() => {
    if (!editor || updatingFromEditor) {
      return;
    }
    const value = editor.getValue();
    if (value !== props.modelValue) {
      emit('update:modelValue', value);
    }
  });
}

watch(
  () => props.modelValue,
  (value) => {
    if (!editor) {
      return;
    }
    const current = editor.getValue();
    if (value !== current) {
      updatingFromEditor = true;
      editor.setValue(value ?? '');
      updatingFromEditor = false;
    }
  }
);

watch(
  () => props.readOnly,
  (value) => {
    if (editor) {
      editor.updateOptions({ readOnly: value ?? false });
    }
  }
);

onMounted(() => {
  void initEditor();
});

onBeforeUnmount(() => {
  if (editor) {
    editor.dispose();
    editor = null;
  }
  monacoApi = null;
});
</script>

<template>
  <div ref="containerRef" class="w-full rounded-md overflow-hidden border border-slate-800" :style="{ height: editorHeight }"></div>
</template>
