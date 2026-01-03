<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import type * as Monaco from 'monaco-editor';
import type { ResolvedTheme } from '../../shared/theme';

const props = defineProps<{
  modelValue: string;
  language?: string;
  readOnly?: boolean;
  height?: string;
  theme?: ResolvedTheme;
}>();

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void;
}>();

const containerRef = ref<HTMLDivElement | null>(null);
const editorHeight = computed(() => props.height ?? '260px');

let editor: Monaco.editor.IStandaloneCodeEditor | null = null;
let monacoApi: typeof Monaco | null = null;
let updatingFromEditor = false;
let tomlRegistered = false;

function resolveMonacoTheme() {
  if (props.theme === 'light') {
    return 'vs';
  }
  if (props.theme === 'darcula') {
    return 'darcula';
  }
  return 'vs-dark';
}

function applyMonacoTheme() {
  if (monacoApi) {
    monacoApi.editor.setTheme(resolveMonacoTheme());
  }
}

function ensureTomlLanguage(monaco: typeof Monaco) {
  if (tomlRegistered) {
    return;
  }
  monaco.languages.register({ id: 'toml' });
  monaco.languages.setMonarchTokensProvider('toml', {
    tokenizer: {
      root: [
        [/^\s*\[[^\]]+\]/, 'tag'],
        [/^\s*#.*$/, 'comment'],
        [/"([^"\\]|\\.)*$/, 'string.invalid'],
        [/"([^"\\]|\\.)*"/, 'string'],
        [/'([^'\\]|\\.)*'/, 'string'],
        [/\b(true|false)\b/, 'keyword'],
        [/[+-]?\d+(\.\d+)?([eE][+-]?\d+)?/, 'number'],
        [/^[A-Za-z0-9_-]+(?=\s*=)/, 'key'],
        [/[{}\[\],=]/, 'delimiter'],
      ],
    },
  });
  monaco.languages.setLanguageConfiguration('toml', {
    comments: { lineComment: '#' },
    brackets: [
      ['[', ']'],
      ['{', '}'],
    ],
    autoClosingPairs: [
      { open: '"', close: '"' },
      { open: "'", close: "'" },
      { open: '[', close: ']' },
      { open: '{', close: '}' },
    ],
    surroundingPairs: [
      { open: '"', close: '"' },
      { open: "'", close: "'" },
      { open: '[', close: ']' },
      { open: '{', close: '}' },
    ],
  });
  tomlRegistered = true;
}

function ensureDarculaTheme(monaco: typeof Monaco) {
  try {
    monaco.editor.defineTheme('darcula', {
      base: 'vs-dark',
      inherit: true,
      rules: [],
      colors: {
        'editor.background': '#2B2B2B',
        'editor.foreground': '#A9B7C6',
        'editorLineNumber.foreground': '#606366',
        'editorCursor.foreground': '#A9B7C6',
        'editor.selectionBackground': '#214283',
        'editor.inactiveSelectionBackground': '#21428380',
        'editorIndentGuide.background': '#3C3F41',
        'editorIndentGuide.activeBackground': '#4E5254',
      },
    });
  } catch {
    // ignore defineTheme errors
  }
}

async function initEditor() {
  if (!containerRef.value || editor) {
    return;
  }
  monacoApi = await import('monaco-editor');
  ensureTomlLanguage(monacoApi);
  ensureDarculaTheme(monacoApi);
  monacoApi.editor.setTheme(resolveMonacoTheme());
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

watch(
  () => props.theme,
  () => {
    applyMonacoTheme();
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
  <div ref="containerRef" class="w-full rounded-md overflow-hidden border border-border" :style="{ height: editorHeight }"></div>
</template>
