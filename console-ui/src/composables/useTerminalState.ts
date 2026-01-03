import { computed, ref, type Ref } from 'vue';
import type { TargetInfo } from '../shared/types';

type TerminalTab = {
  id: string;
  label: string;
  createdAt: number;
};

type TerminalTargetState = {
  open: boolean;
  tabs: TerminalTab[];
  activeId: string | null;
  nextIndex: number;
};

type TerminalEntry = {
  target: TargetInfo;
  state: TerminalTargetState;
};

type UseTerminalStateOptions = {
  selectedTargetName: Ref<string | null>;
  targets: Ref<TargetInfo[]>;
};

export function useTerminalState({ selectedTargetName, targets }: UseTerminalStateOptions) {
  const terminalState = ref<Record<string, TerminalTargetState>>({});

  const selectedTerminal = computed<TerminalTargetState>(() => {
    if (!selectedTargetName.value) {
      return { open: false, tabs: [], activeId: null, nextIndex: 1 };
    }
    return (
      terminalState.value[selectedTargetName.value] ?? {
        open: false,
        tabs: [],
        activeId: null,
        nextIndex: 1,
      }
    );
  });

  const selectedTerminalOpen = computed(
    () => selectedTerminal.value.open && selectedTerminal.value.tabs.length > 0
  );

  const terminalEntries = computed<TerminalEntry[]>(() =>
    targets.value
      .map((target) => ({ target, state: terminalState.value[target.name] }))
      .filter((entry): entry is TerminalEntry => Boolean(entry.state && entry.state.tabs.length > 0))
      .map((entry) => ({ target: entry.target, state: entry.state }))
  );

  const selectedTerminalEntry = computed<TerminalEntry | null>(() => {
    if (!selectedTargetName.value) {
      return null;
    }
    return terminalEntries.value.find((item) => item.target.name === selectedTargetName.value) ?? null;
  });

  const activeTerminalTabId = computed<string | number | undefined>(() => {
    return selectedTerminalEntry.value?.state.activeId ?? undefined;
  });

  function createTabId() {
    if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
      return crypto.randomUUID();
    }
    return `term-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  }

  function createTerminalTab(index: number): TerminalTab {
    return {
      id: createTabId(),
      label: `Session ${index}`,
      createdAt: Date.now(),
    };
  }

  function setTerminalState(name: string, state: TerminalTargetState) {
    terminalState.value = {
      ...terminalState.value,
      [name]: state,
    };
  }

  function openTerminalForTarget(name: string) {
    const current = terminalState.value[name];
    if (!current) {
      const tab = createTerminalTab(1);
      setTerminalState(name, { open: true, tabs: [tab], activeId: tab.id, nextIndex: 2 });
      return;
    }
    if (current.tabs.length === 0) {
      const tab = createTerminalTab(current.nextIndex || 1);
      setTerminalState(name, {
        ...current,
        open: true,
        tabs: [tab],
        activeId: tab.id,
        nextIndex: (current.nextIndex || 1) + 1,
      });
      return;
    }
    if (!current.open) {
      setTerminalState(name, { ...current, open: true });
    }
  }

  function hideTerminalForTarget(name: string) {
    const current = terminalState.value[name];
    if (!current) {
      return;
    }
    if (current.open) {
      setTerminalState(name, { ...current, open: false });
    }
  }

  function addTerminalTab(name: string) {
    const current = terminalState.value[name] ?? {
      open: true,
      tabs: [],
      activeId: null,
      nextIndex: 1,
    };
    const tab = createTerminalTab(current.nextIndex || 1);
    setTerminalState(name, {
      ...current,
      open: true,
      tabs: [...current.tabs, tab],
      activeId: tab.id,
      nextIndex: (current.nextIndex || 1) + 1,
    });
  }

  function activateTerminalTab(name: string, tabId: string) {
    const current = terminalState.value[name];
    if (!current || current.activeId === tabId) {
      return;
    }
    setTerminalState(name, { ...current, activeId: tabId, open: true });
  }

  function closeTerminalTab(name: string, tabId: string) {
    const current = terminalState.value[name];
    if (!current) {
      return;
    }
    const index = current.tabs.findIndex((tab) => tab.id === tabId);
    if (index === -1) {
      return;
    }
    const nextTabs = current.tabs.filter((tab) => tab.id !== tabId);
    if (nextTabs.length === 0) {
      setTerminalState(name, {
        ...current,
        open: false,
        tabs: [],
        activeId: null,
      });
      return;
    }
    const nextActiveId =
      current.activeId === tabId ? nextTabs[Math.min(index, nextTabs.length - 1)].id : current.activeId;
    setTerminalState(name, {
      ...current,
      tabs: nextTabs,
      activeId: nextActiveId ?? nextTabs[0].id,
    });
  }

  function openSelectedTerminal() {
    if (!selectedTargetName.value) {
      return;
    }
    openTerminalForTarget(selectedTargetName.value);
  }

  function closeSelectedTerminal() {
    if (!selectedTargetName.value) {
      return;
    }
    hideTerminalForTarget(selectedTargetName.value);
  }

  function handleAddTerminalTab() {
    const entry = selectedTerminalEntry.value;
    if (!entry) {
      return;
    }
    addTerminalTab(entry.target.name);
  }

  function handleCloseTerminalTab(name: string | number) {
    const entry = selectedTerminalEntry.value;
    if (!entry) {
      return;
    }
    closeTerminalTab(entry.target.name, String(name));
  }

  function handleActivateTerminalTab(value: string | number) {
    const entry = selectedTerminalEntry.value;
    if (!entry) {
      return;
    }
    activateTerminalTab(entry.target.name, String(value));
  }

  return {
    activeTerminalTabId,
    closeSelectedTerminal,
    handleActivateTerminalTab,
    handleAddTerminalTab,
    handleCloseTerminalTab,
    openSelectedTerminal,
    selectedTerminalEntry,
    selectedTerminalOpen,
    terminalEntries,
  };
}
