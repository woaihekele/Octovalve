<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { NButton, NCard, NInput, NModal, NProgress, NSpin, NTree, type TreeOption } from 'naive-ui';
import { useI18n } from 'vue-i18n';
import { isTauri } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { fetchUploadStatus, listTargetDirectories, startUpload } from '../../services/api';
import type { TargetInfo, UploadStatus } from '../../shared/types';

const props = defineProps<{
  show: boolean;
  target: TargetInfo | null;
}>();

const emit = defineEmits<{
  (e: 'update:show', value: boolean): void;
}>();

const { t } = useI18n();
const tauriAvailable = isTauri();
const treeData = ref<TreeOption[]>([]);
const expandedKeys = ref<Array<string | number>>([]);
const selectedKeys = ref<Array<string | number>>([]);
const treeLoading = ref(false);
const localFilePath = ref('');
const fileName = ref('');
const uploadId = ref<string | null>(null);
const uploadStatus = ref<UploadStatus | null>(null);
const uploadError = ref<string | null>(null);
let pollTimer: number | null = null;

const selectedDir = computed(() => {
  const key = selectedKeys.value[0];
  return typeof key === 'string' ? key : '/';
});

const remotePath = computed(() => buildRemotePath(selectedDir.value, fileName.value));

const uploadBusy = computed(() => {
  const status = uploadStatus.value?.status;
  return status === 'pending' || status === 'running';
});

const progressStatus = computed(() => {
  const status = uploadStatus.value?.status;
  if (status === 'failed') {
    return 'error';
  }
  if (status === 'completed') {
    return 'success';
  }
  return 'default';
});

const progressPercentage = computed(() => {
  if (!uploadStatus.value || uploadStatus.value.total_bytes <= 0) {
    return 0;
  }
  const ratio = uploadStatus.value.sent_bytes / uploadStatus.value.total_bytes;
  return Math.min(100, Math.round(ratio * 100));
});

const progressLabel = computed(() => {
  if (!uploadStatus.value || uploadStatus.value.total_bytes <= 0) {
    return '';
  }
  return `${formatBytes(uploadStatus.value.sent_bytes)} / ${formatBytes(uploadStatus.value.total_bytes)}`;
});

const statusLabel = computed(() => {
  if (!uploadStatus.value) {
    return '';
  }
  switch (uploadStatus.value.status) {
    case 'pending':
      return t('terminal.upload.status.pending');
    case 'running':
      return t('terminal.upload.status.running');
    case 'completed':
      return t('terminal.upload.status.completed');
    case 'failed':
      return t('terminal.upload.status.failed');
    default:
      return '';
  }
});

watch(
  () => props.show,
  (show) => {
    if (show) {
      void initializeModal();
    } else {
      stopPolling();
    }
  }
);

watch(
  () => props.target?.name,
  () => {
    if (props.show) {
      void initializeModal();
    }
  }
);

watch(
  () => selectedKeys.value,
  (keys) => {
    const dir = keys[0];
    if (!props.target || typeof dir !== 'string') {
      return;
    }
    localStorage.setItem(storageKey(props.target.name), dir);
  }
);

watch(
  () => uploadStatus.value,
  (status) => {
    if (!status) {
      return;
    }
    if (status.status === 'failed' && status.error) {
      uploadError.value = status.error;
    }
  }
);

onBeforeUnmount(() => {
  stopPolling();
});

async function initializeModal() {
  uploadError.value = null;
  uploadStatus.value = null;
  uploadId.value = null;
  localFilePath.value = '';
  fileName.value = '';
  treeLoading.value = true;
  treeData.value = [buildRootNode()];
  expandedKeys.value = ['/'];
  selectedKeys.value = ['/'];

  if (!props.target) {
    treeLoading.value = false;
    return;
  }

  try {
    await loadNodeChildren(treeData.value[0]);
    await restoreSelection(props.target.name);
  } catch (err) {
    uploadError.value = String(err);
  } finally {
    treeLoading.value = false;
  }
  await nextTick();
}

function buildRootNode(): TreeOption {
  return {
    key: '/',
    label: '/',
    children: undefined,
    isLeaf: false,
  };
}

async function restoreSelection(targetName: string) {
  const saved = localStorage.getItem(storageKey(targetName));
  if (!saved) {
    selectedKeys.value = ['/'];
    return;
  }
  const segments = saved.split('/').filter(Boolean);
  let currentPath = '/';
  let currentNode = treeData.value[0];
  const nextExpanded: Array<string | number> = ['/'];
  for (const segment of segments) {
    if (!currentNode) {
      break;
    }
    await ensureChildrenLoaded(currentNode);
    const children = currentNode.children ?? [];
    const nextPath = joinRemotePath(currentPath, segment);
    const nextNode = children.find((child) => child.key === nextPath) as TreeOption | undefined;
    if (!nextNode) {
      break;
    }
    nextExpanded.push(nextPath);
    currentPath = nextPath;
    currentNode = nextNode;
  }
  expandedKeys.value = nextExpanded;
  selectedKeys.value = [currentPath];
}

async function ensureChildrenLoaded(node: TreeOption) {
  if (node.children !== undefined) {
    return;
  }
  await loadNodeChildren(node);
}

async function loadNodeChildren(node: TreeOption) {
  if (!props.target) {
    return;
  }
  if (!tauriAvailable) {
    throw new Error(t('api.tauriOnly.upload'));
  }
  const path = typeof node.key === 'string' ? node.key : '/';
  const listing = await listTargetDirectories(props.target.name, path);
  const children = listing.entries.map((entry) => ({
    key: entry.path,
    label: entry.name,
    children: undefined,
    isLeaf: false,
  }));
  node.children = children;
  node.isLeaf = children.length === 0;
}

function handleExpandedKeys(keys: Array<string | number>) {
  expandedKeys.value = keys;
}

function handleSelectedKeys(keys: Array<string | number>) {
  selectedKeys.value = keys;
}

async function handleChooseFile() {
  uploadError.value = null;
  if (!tauriAvailable) {
    uploadError.value = t('api.tauriOnly.upload');
    return;
  }
  const selected = await open({
    directory: false,
    multiple: false,
  });
  const path = Array.isArray(selected) ? selected[0] : selected;
  if (!path || typeof path !== 'string') {
    return;
  }
  localFilePath.value = path;
  fileName.value = extractFileName(path);
}

async function handleUpload() {
  uploadError.value = null;
  if (!props.target) {
    return;
  }
  if (!localFilePath.value) {
    uploadError.value = t('terminal.upload.errorNoFile');
    return;
  }
  if (!fileName.value.trim()) {
    uploadError.value = t('terminal.upload.errorNoName');
    return;
  }
  try {
    const response = await startUpload(props.target.name, localFilePath.value, remotePath.value);
    uploadId.value = response.id;
    uploadStatus.value = {
      id: response.id,
      target: props.target.name,
      local_path: localFilePath.value,
      remote_path: remotePath.value,
      status: 'pending',
      total_bytes: 0,
      sent_bytes: 0,
    };
    startPolling();
  } catch (err) {
    uploadError.value = String(err);
  }
}

function startPolling() {
  stopPolling();
  const id = uploadId.value;
  if (!id) {
    return;
  }
  const poll = async () => {
    try {
      const status = await fetchUploadStatus(id);
      uploadStatus.value = status;
      if (status.status === 'completed' || status.status === 'failed') {
        stopPolling();
      }
    } catch (err) {
      uploadError.value = String(err);
      stopPolling();
    }
  };
  void poll();
  pollTimer = window.setInterval(poll, 800);
}

function stopPolling() {
  if (pollTimer !== null) {
    window.clearInterval(pollTimer);
    pollTimer = null;
  }
}

function closeModal() {
  if (uploadBusy.value) {
    return;
  }
  emit('update:show', false);
}

function storageKey(targetName: string) {
  return `console-ui.upload.last-dir.${targetName}`;
}

function joinRemotePath(base: string, name: string) {
  if (base === '/') {
    return `/${name}`;
  }
  if (base.endsWith('/')) {
    return `${base}${name}`;
  }
  return `${base}/${name}`;
}

function buildRemotePath(base: string, name: string) {
  const trimmedName = name.trim();
  if (!trimmedName) {
    return base || '/';
  }
  if (base === '/') {
    return `/${trimmedName}`;
  }
  if (base.endsWith('/')) {
    return `${base}${trimmedName}`;
  }
  return `${base}/${trimmedName}`;
}

function extractFileName(path: string) {
  const normalized = path.replace(/\\/g, '/');
  const parts = normalized.split('/');
  return parts[parts.length - 1] || normalized;
}

function formatBytes(value: number) {
  if (!Number.isFinite(value)) {
    return '-';
  }
  if (value < 1024) {
    return `${value} B`;
  }
  const units = ['KB', 'MB', 'GB', 'TB'];
  let size = value;
  let unitIndex = -1;
  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex += 1;
  }
  const precision = size >= 100 ? 0 : 1;
  return `${size.toFixed(precision)} ${units[unitIndex]}`;
}
</script>

<template>
  <n-modal
    :show="props.show"
    :mask-closable="!uploadBusy"
    :close-on-esc="!uploadBusy"
    @update:show="(value) => emit('update:show', value)"
  >
    <n-card class="w-[720px]" :bordered="true">
      <template #header>
        {{ t('terminal.upload.title') }}
      </template>
      <template #header-extra>
        <n-button text :disabled="uploadBusy" @click="closeModal">
          {{ t('common.close') }}
        </n-button>
      </template>

      <div class="space-y-4">
        <div>
          <div class="text-sm font-medium text-foreground mb-2">{{ t('terminal.upload.remoteDir') }}</div>
          <div class="border border-border rounded-md p-2 bg-panel/40 min-h-[180px]">
            <n-spin :show="treeLoading" size="small">
              <n-tree
                :data="treeData"
                :expanded-keys="expandedKeys"
                :selected-keys="selectedKeys"
                :on-load="loadNodeChildren"
                @update:expanded-keys="handleExpandedKeys"
                @update:selected-keys="handleSelectedKeys"
              />
            </n-spin>
          </div>
        </div>

        <div>
          <div class="text-sm font-medium text-foreground mb-2">{{ t('terminal.upload.localFile') }}</div>
          <div class="flex items-center gap-2">
            <n-input :value="localFilePath" readonly />
            <n-button @click="handleChooseFile">{{ t('terminal.upload.selectFile') }}</n-button>
          </div>
        </div>

        <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
          <div>
            <div class="text-sm font-medium text-foreground mb-2">{{ t('terminal.upload.fileName') }}</div>
            <n-input v-model:value="fileName" />
          </div>
          <div>
            <div class="text-sm font-medium text-foreground mb-2">{{ t('terminal.upload.remotePath') }}</div>
            <n-input :value="remotePath" readonly />
          </div>
        </div>

        <div v-if="uploadStatus" class="space-y-2">
          <n-progress
            :percentage="progressPercentage"
            :status="progressStatus"
            :processing="uploadBusy"
          />
          <div class="text-xs text-foreground-muted flex justify-between">
            <span>{{ statusLabel }}</span>
            <span>{{ progressLabel }}</span>
          </div>
        </div>

        <div v-if="uploadError" class="text-xs text-danger">
          {{ uploadError }}
        </div>
      </div>

      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button :disabled="uploadBusy" @click="closeModal">{{ t('common.cancel') }}</n-button>
          <n-button type="primary" :disabled="uploadBusy" @click="handleUpload">
            {{ t('terminal.upload.action') }}
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>
</template>
