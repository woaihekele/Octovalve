<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, provide } from 'vue';
import { NConfigProvider, NNotificationProvider, darkTheme } from 'naive-ui';
import ConsoleView from './ConsoleView.vue';
import { useThemeMode } from '../composables/useThemeMode';
import { APPLY_THEME_MODE, RESOLVED_THEME } from './appContext';
import { createNaiveThemeOverrides } from '../shared/naiveTheme';

const { resolvedTheme, applyThemeMode } = useThemeMode();
provide(APPLY_THEME_MODE, applyThemeMode);
provide(RESOLVED_THEME, resolvedTheme);

const naiveTheme = computed(() => (resolvedTheme.value === 'light' ? null : darkTheme));

const naiveThemeOverrides = computed(() => createNaiveThemeOverrides(resolvedTheme.value));

const fileDropListenerOptions = { capture: true };

function isFileDrag(event: DragEvent) {
  const items = event.dataTransfer?.items;
  if (items) {
    for (const item of Array.from(items)) {
      if (item.kind === 'file') {
        return true;
      }
    }
  }
  const types = event.dataTransfer?.types;
  if (!types) {
    return false;
  }
  return Array.from(types).includes('Files');
}

function handleGlobalDragOver(event: DragEvent) {
  if (isFileDrag(event)) {
    event.preventDefault();
  }
}

function handleGlobalDrop(event: DragEvent) {
  if (isFileDrag(event)) {
    event.preventDefault();
  }
}

onMounted(() => {
  window.addEventListener('dragover', handleGlobalDragOver, fileDropListenerOptions);
  window.addEventListener('drop', handleGlobalDrop, fileDropListenerOptions);
});

onBeforeUnmount(() => {
  window.removeEventListener('dragover', handleGlobalDragOver, fileDropListenerOptions);
  window.removeEventListener('drop', handleGlobalDrop, fileDropListenerOptions);
});
</script>

<template>
  <n-config-provider :theme="naiveTheme" :theme-overrides="naiveThemeOverrides">
    <n-notification-provider>
      <ConsoleView />
    </n-notification-provider>
  </n-config-provider>
</template>
