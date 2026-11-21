// UI state shared across views. Persistent data (hosts) lives in the Rust
// core; this store only caches what the UI shows (no secrets).
import { create } from "zustand";
import { errorText, HostView, hostsList, ShellRequest } from "./ipc";

export type ViewId = "overview" | "hosts" | "devices" | "device" | "desktop" | "deploy" | "automations" | "sessions" | "files" | "workspaces" | "diagnostics";

export interface Tab {
  key: number;
  title: string;
  shell: ShellRequest;
  exited: boolean;
}

interface UiState {
  view: ViewId;
  setView: (v: ViewId) => void;

  tabs: Tab[];
  activeKey: number | null;
  openTab: (title: string, shell: ShellRequest) => void;
  closeTab: (key: number) => void;
  setActive: (key: number) => void;
  markExited: (key: number) => void;
  /** Replace an ended tab with a fresh session of the same kind (same tmux session if any). */
  reopenTab: (key: number) => void;

  hosts: HostView[];
  hostsError: string | null;
  refreshHosts: () => Promise<void>;
  putHost: (h: HostView) => void;

  filesHostId: string | null;
  openFiles: (hostId: string) => void;

  deviceRouteId: string | null;
  openDevice: (routeId: string) => void;

  desktopHostId: string | null;
  openDesktop: (hostId: string) => void;
}

let nextKey = 1;

export const useUi = create<UiState>((set, get) => ({
  view: "sessions",
  setView: (view) => set({ view }),

  tabs: [],
  activeKey: null,
  openTab: (title, shell) => {
    const tab = { key: nextKey++, title, shell, exited: false };
    set((s) => ({ tabs: [...s.tabs, tab], activeKey: tab.key, view: "sessions" }));
  },
  closeTab: (key) => {
    const rest = get().tabs.filter((t) => t.key !== key);
    set((s) => ({ tabs: rest, activeKey: s.activeKey === key ? (rest.at(-1)?.key ?? null) : s.activeKey }));
  },
  setActive: (key) => set({ activeKey: key }),
  markExited: (key) => set((s) => ({ tabs: s.tabs.map((t) => (t.key === key ? { ...t, exited: true } : t)) })),
  reopenTab: (key) =>
    set((s) => {
      const fresh = nextKey++;
      return {
        tabs: s.tabs.map((t) => (t.key === key ? { ...t, key: fresh, exited: false } : t)),
        activeKey: s.activeKey === key ? fresh : s.activeKey,
      };
    }),

  hosts: [],
  hostsError: null,
  refreshHosts: async () => {
    try {
      set({ hosts: await hostsList(), hostsError: null });
    } catch (e) {
      set({ hostsError: errorText(e) });
    }
  },
  putHost: (h) => set((s) => ({ hosts: s.hosts.map((x) => (x.id === h.id ? h : x)) })),

  filesHostId: null,
  openFiles: (hostId) => set({ filesHostId: hostId, view: "files" }),

  deviceRouteId: null,
  openDevice: (routeId) => set({ deviceRouteId: routeId, view: "device" }),

  desktopHostId: null,
  openDesktop: (hostId) => set({ desktopHostId: hostId, view: "desktop" }),
}));
