<script setup lang="ts">
const props = defineProps<{
  message: string;
  count?: number;
}>();
</script>

<template>
  <div class="fixed top-6 right-6 z-50 bg-slate-800 border border-slate-700 text-white px-4 py-3 rounded-lg shadow-xl flex items-center gap-3">
    <div
      class="h-8 w-8 rounded-full flex items-center justify-center"
      :class="(props.count ?? 0) > 0 ? 'bg-rose-500/20 text-rose-300' : 'bg-emerald-500/20 text-emerald-300'"
    >
      <span class="text-xs font-semibold">!</span>
    </div>
    <div>
      <p class="font-medium text-sm">{{ props.message }}</p>
      <p v-if="(props.count ?? 0) > 0" class="text-xs text-slate-400">{{ props.count }} 个待审批</p>
    </div>
  </div>
</template>
