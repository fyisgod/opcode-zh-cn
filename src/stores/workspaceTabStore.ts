import { create } from "zustand";
import { persist } from "zustand/middleware";

export const workspaceModules = [
  { id: "overview", label: "Session overview", tabLabel: "Overview" },
  { id: "notices", label: "Warnings and notices", tabLabel: "Notices" },
  { id: "files", label: "Files", tabLabel: "Files" },
  { id: "git", label: "Git", tabLabel: "Git" },
  { id: "agents", label: "Session agents", tabLabel: "Session agents" },
  { id: "tasks", label: "Background tasks", tabLabel: "Background tasks" },
] as const;

export type WorkspaceModule = typeof workspaceModules[number]["id"];

export type WorkspaceTab = {
  id: string;
  name?: string;
  modules: WorkspaceModule[];
};

type WorkspaceTabStore = {
  tabs: WorkspaceTab[];
  saveTab: (tab: WorkspaceTab) => void;
  deleteTab: (id: string) => void;
};

export const useWorkspaceTabs = create<WorkspaceTabStore>()(persist((set) => ({
  tabs: [],
  saveTab: (tab) => set(({ tabs }) => ({
    tabs: tabs.some(({ id }) => id === tab.id)
      ? tabs.map((existing) => existing.id === tab.id ? tab : existing)
      : [...tabs, tab],
  })),
  deleteTab: (id) => set(({ tabs }) => ({
    tabs: tabs.filter((tab) => tab.id !== id),
  })),
}), {
  name: "opcode-workspace-tabs",
  version: 1,
  migrate: (state) => {
    const oldTabs = (state as { tabs?: WorkspaceTab[] } | undefined)?.tabs ?? [];
    return { tabs: oldTabs.filter((tab) => tab.id !== "session" || tab.name) };
  },
}));
