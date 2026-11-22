import { useLayoutEffect, useId, useRef, useState, type Dispatch, type ReactNode, type SetStateAction } from "react";
import { Bell, Bot, FolderOpen, GitBranch, Info, ListTodo, PanelRightClose, Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator,
  DropdownMenuSub, DropdownMenuSubContent, DropdownMenuSubTrigger, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useWorkspaceTabs, workspaceModules, type WorkspaceModule, type WorkspaceTab } from "@/stores/workspaceTabStore";
import { Root as Tabs, Content as TabsContent, List as TabsList, Trigger as TabsTrigger } from "@radix-ui/react-tabs";
import { t } from "@/lib/i18n";
import { cn } from "@/lib/utils";
import { WorkspaceEmpty, WorkspaceFiles, WorkspaceGit, type WorkspaceBrowserProps } from "./WorkspaceBrowser";
import { type readSessionWorkspace, type RunState, type WorkspaceNotice } from "./workspaceActivity";

type SessionData = ReturnType<typeof readSessionWorkspace>;

export type OpenWorkspaceTabs = { open: WorkspaceModule[]; active: WorkspaceModule | "" };

type Props = WorkspaceBrowserProps & {
  id: string;
  tabs: OpenWorkspaceTabs;
  setTabs: Dispatch<SetStateAction<OpenWorkspaceTabs>>;
  onClose: () => void;
  session: SessionData;
  notices: WorkspaceNotice[];
  sessionId?: string | null;
  model?: string;
  running: boolean;
};

function Section({ title, children }: { title: string; children: ReactNode }) {
  return <section className="border-b border-border px-3 py-3 last:border-none">
    <h3 className="mb-2 text-xs font-semibold text-foreground">{t(title)}</h3>
    {children}
  </section>;
}

function Field({ label, value }: { label: string; value?: string | number | null }) {
  if (value == null || value === "") return null;
  return <div className="grid grid-cols-[6rem_minmax(0,1fr)] gap-2 py-1 text-xs leading-relaxed">
    <dt className="text-muted-foreground">{t(label)}</dt>
    <dd className="select-text break-all" title={String(value)}>{value}</dd>
  </div>;
}

function State({ state }: { state: RunState | "Ready" }) {
  return <span className={cn("inline-flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground", state === "Failed" && "text-destructive")}>
    <span aria-hidden="true" className={cn("size-1.5 rounded-full bg-current", state === "Running" && "bg-primary", state === "Completed" && "bg-green-500")} />
    {t(state)}
  </span>;
}

function Overview({ session, root, sessionId, model }: Pick<Props, "session" | "root" | "sessionId" | "model">) {
  const { init, result } = session;
  const tools = Array.isArray(init?.tools) ? init.tools.filter((tool): tool is string => typeof tool === "string") : [];
  const cost = result?.total_cost_usd ?? result?.cost_usd;
  return <>
    <Section title={init ? "System Initialized" : "Session details"}>
      <dl>
        <Field label="Session ID" value={init?.session_id || sessionId} />
        <Field label="Model" value={init?.model || model} />
        <Field label="Working directory" value={init?.cwd || root} />
        <Field label="Permission mode" value={init?.permissionMode} />
        <Field label="CLI version" value={init?.claude_code_version} />
      </dl>
    </Section>
    {tools.length > 0 && <Section title="Available tools">
      <ul className="flex flex-wrap gap-1.5">{tools.map((tool, index) => <li key={`${tool}-${index}`} className="max-w-full break-all rounded-md border border-border bg-muted/60 px-2 py-0.5 text-xs text-muted-foreground">{tool}</li>)}</ul>
    </Section>}
    {result && <Section title="Last run">
      <dl>
        <Field label="Status" value={t(result.is_error || result.subtype?.startsWith("error") ? "Failed" : "Completed")} />
        <Field label="Duration" value={typeof result.duration_ms === "number" ? `${(result.duration_ms / 1000).toFixed(2)} s` : undefined} />
        <Field label="Turns" value={result.num_turns} />
        <Field label="Input tokens" value={result.usage?.input_tokens?.toLocaleString()} />
        <Field label="Output tokens" value={result.usage?.output_tokens?.toLocaleString()} />
        <Field label="Cost" value={typeof cost === "number" ? `$${cost.toFixed(4)} USD` : undefined} />
      </dl>
    </Section>}
  </>;
}

function Notices({ notices }: { notices: WorkspaceNotice[] }) {
  if (!notices.length) return <WorkspaceEmpty>{t("No warnings or notices")}</WorkspaceEmpty>;
  return <ol className="divide-y divide-border">
    {[...notices].reverse().map(({ level, text, time }, index) => <li className="space-y-1 px-3 py-3" key={index}>
      <div className="flex justify-between gap-2 text-xs">
        <span className={cn("font-medium text-muted-foreground", level === "Warning" && "text-amber-700 dark:text-amber-400", level === "Error" && "text-destructive")}>{t(level)}</span>
        {time && <time className="text-muted-foreground" dateTime={new Date(time).toISOString()}>{new Date(time).toLocaleTimeString()}</time>}
      </div>
      <p className="select-text whitespace-pre-wrap break-words text-xs leading-relaxed">{text}</p>
    </li>)}
  </ol>;
}

function RunActivity({ session, running, model, section }: Pick<Props, "session" | "running" | "model"> & { section: "agents" | "tasks" }) {
  const agents = session.activity.filter((item) => item.kind === "agent");
  const latest = session.result;
  const mainState = running ? "Running" : latest ? (latest.is_error || latest.subtype?.startsWith("error") ? "Failed" : "Completed") : "Ready";
  const agentsById = new Map(agents.map((agent) => [agent.id, agent.title]));
  function rows(items: typeof agents, empty: string) {
    if (!items.length) return <p className="text-xs text-muted-foreground">{t(empty)}</p>;
    return <ul className="space-y-1.5">{items.map((item) => {
      const parent = item.parent ? agentsById.get(item.parent) : undefined;
      const state = !item.background && item.state === "Running" && !running ? "Pending" : item.state;
      return <li key={item.id}>
        <details className="rounded-md border border-border bg-background/50">
          <summary className="cursor-pointer rounded-md px-2 py-2 text-xs transition-colors hover:bg-accent">
            <span className="inline-flex w-[calc(100%-1.25rem)] items-center justify-between gap-2 align-middle">
              <span className="truncate font-medium" title={item.title}>{item.title}</span>
              <State state={state} />
            </span>
          </summary>
          <div className="space-y-2 border-t border-border p-2 text-xs">
            <dl>
              <Field label="Agent type" value={item.agentType} />
              <Field label="Parent agent" value={parent} />
              <Field label={item.taskId && item.taskId === item.id ? "Task ID" : "Tool call ID"} value={item.id} />
              {item.taskId && item.taskId !== item.id && <Field label="Task ID" value={item.taskId} />}
            </dl>
            {item.details && <p className="select-text whitespace-pre-wrap break-words leading-relaxed">{item.details}</p>}
          </div>
        </details>
      </li>;
    })}</ul>;
  }
  if (section === "tasks") {
    const tasks = session.activity.filter((item) => item.kind === "task" || item.background);
    return <div className="px-3 py-3">{rows(tasks, "No background tasks in this session")}</div>;
  }
  return <>
    <Section title="Main agent">
      <div className="flex items-center gap-2 rounded-md border border-border bg-background/50 px-2 py-2">
        <Bot size={16} className="shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-xs">{session.init?.model || model || "Claude Code"}</span>
        <State state={mainState} />
      </div>
    </Section>
    <Section title="Subagents">{rows(agents, "No subagents in this session")}</Section>
  </>;
}

function WorkspaceTabEditor({ tab, onSave, onDelete, onClose, returnFocus }: {
  tab: WorkspaceTab | null;
  onSave: (tab: WorkspaceTab) => void;
  onDelete?: () => void;
  onClose: () => void;
  returnFocus: () => void;
}) {
  const nameId = useId();
  const [name, setName] = useState(tab?.name ?? "");
  const [modules, setModules] = useState<WorkspaceModule[]>(tab?.modules ?? []);
  const selectedModules = workspaceModules.map(({ id }) => id).filter((id) => modules.includes(id));

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent aria-describedby={undefined} className="max-h-[90vh] w-[calc(100%-2rem)] max-w-sm overflow-y-auto rounded-xl p-5" onCloseAutoFocus={(event) => {
        event.preventDefault();
        returnFocus();
      }}>
        <DialogHeader className="text-left">
          <DialogTitle className="text-base">{t(tab ? "Edit tab group" : "New tab group")}</DialogTitle>
        </DialogHeader>
        <form className="space-y-4" onSubmit={(event) => {
          event.preventDefault();
          if (!name.trim() || selectedModules.length === 0) return;
          onSave({ id: tab?.id ?? crypto.randomUUID(), name: name.trim(), modules: selectedModules });
        }}>
          <div className="space-y-2">
            <Label htmlFor={nameId} className="text-xs">{t("Group name")}</Label>
            <Input id={nameId} value={name} onChange={(event) => setName(event.target.value)} maxLength={40} required className="h-8 text-sm" />
          </div>
          <fieldset className="space-y-1">
            <legend className="mb-2 text-xs font-medium">{t("Tabs to open")}</legend>
            {workspaceModules.map(({ id, label }) => (
              <label key={id} className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-xs transition-colors hover:bg-accent">
                <input type="checkbox" className="size-4 accent-primary" checked={modules.includes(id)} onChange={(event) => {
                  const checked = event.target.checked;
                  setModules((current) => checked ? [...current, id] : current.filter((item) => item !== id));
                }} />
                {t(label)}
              </label>
            ))}
          </fieldset>
          <DialogFooter className="flex-row items-center justify-end gap-1 sm:space-x-0">
            {onDelete && <Button type="button" variant="ghost" size="sm" className="mr-auto text-destructive" onClick={onDelete}>{t("Delete group")}</Button>}
            <Button type="button" variant="ghost" size="sm" onClick={onClose}>{t("Cancel")}</Button>
            <Button type="submit" size="sm" disabled={!name.trim() || !selectedModules.length}>{t(tab ? "Save" : "Create group")}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

type ModuleProps = Pick<Props, "session" | "root" | "revision" | "sessionId" | "model" | "running" | "notices">;

function ModuleContents({ module, ...props }: ModuleProps & { module: WorkspaceModule }) {
  const browser = { root: props.root, revision: props.revision };
  switch (module) {
    case "overview": return <Overview {...props} />;
    case "notices": return <Notices notices={props.notices} />;
    case "files": return <WorkspaceFiles key={browser.root} {...browser} />;
    case "git": return <WorkspaceGit key={browser.root} {...browser} />;
    case "agents": return <RunActivity {...props} section="agents" />;
    case "tasks": return <RunActivity {...props} section="tasks" />;
  }
}

const tabIcons = {
  overview: Info,
  notices: Bell,
  files: FolderOpen,
  git: GitBranch,
  agents: Bot,
  tasks: ListTodo,
};

export function SessionWorkspacePanel({ id, onClose, tabs, setTabs, ...props }: Props) {
  const groups = useWorkspaceTabs((state) => state.tabs);
  const saveGroup = useWorkspaceTabs((state) => state.saveTab);
  const deleteGroup = useWorkspaceTabs((state) => state.deleteTab);
  const [editor, setEditor] = useState<{ group: WorkspaceTab | null }>();
  const tabList = useRef<HTMLDivElement>(null);
  const newTabButton = useRef<HTMLButtonElement>(null);
  const focusAfterClose = useRef(false);
  const openingEditor = useRef(false);
  const { open, active } = tabs;

  useLayoutEffect(() => {
    const currentTab = tabList.current?.querySelector<HTMLButtonElement>('[aria-selected="true"]');
    currentTab?.scrollIntoView({ block: "nearest", inline: "nearest" });
    if (focusAfterClose.current) {
      (currentTab ?? newTabButton.current)?.focus({ preventScroll: true });
      focusAfterClose.current = false;
    }
  }, [active, open]);

  function openModules(modules: WorkspaceModule[]) {
    const available = modules.filter((module) => workspaceModules.some(({ id }) => id === module));
    if (available.length === 0) return;
    setTabs((current) => ({ open: [...new Set([...current.open, ...available])], active: available[0] }));
  }

  function closeTab(module: WorkspaceModule) {
    focusAfterClose.current = true;
    setTabs((current) => {
      const index = current.open.indexOf(module);
      const remaining = current.open.filter((item) => item !== module);
      return {
        open: remaining,
        active: current.active === module ? remaining[index] ?? remaining[index - 1] ?? "" : current.active,
      };
    });
  }

  function editGroup(group: WorkspaceTab | null) {
    openingEditor.current = true;
    setEditor({ group });
  }

  return (
    <aside
      id={id}
      aria-label={t("Workspace panel")}
      onKeyDown={(event) => {
        if (event.key === "Escape" && !editor && !event.defaultPrevented) {
          event.stopPropagation();
          onClose();
        }
      }}
      className="absolute inset-y-0 right-0 z-40 flex w-[min(100%,26rem)] shrink-0 flex-col overflow-hidden border-l border-border bg-card shadow-xl @min-[820px]:relative @min-[820px]:w-[clamp(22rem,43cqw,52rem)] @min-[820px]:shadow-none"
    >
      <Tabs
        value={active}
        onValueChange={(value) => setTabs((current) => ({ ...current, active: value as WorkspaceModule }))}
        activationMode="manual"
        className="flex min-h-0 flex-1 flex-col"
      >
        <div className="flex shrink-0 items-center gap-1.5 border-b border-border px-2">
          <TabsList ref={tabList} aria-label={t("Workspace tabs")} className="scrollbar-thin flex min-h-10 min-w-0 flex-1 items-center gap-1 overflow-x-auto py-1">
            {open.map((module) => {
              const definition = workspaceModules.find(({ id }) => id === module)!;
              const Icon = tabIcons[module];
              const name = t(definition.tabLabel);
              const closeLabel = t("Close workspace tab: {name}", { name });
              return (
                <div key={module} role="presentation" className={cn(
                  "flex max-w-44 shrink-0 items-center rounded-md border border-transparent pr-0.5 text-muted-foreground transition-colors hover:bg-accent/60",
                  active === module && "border-border bg-accent text-foreground",
                )}>
                  <TabsTrigger
                    value={module}
                    title={name}
                    className="flex h-7 min-w-0 items-center gap-1.5 rounded-md px-2 text-xs focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
                    onFocus={(event) => event.currentTarget.scrollIntoView({ block: "nearest", inline: "nearest" })}
                    onKeyDown={(event) => {
                      if (event.key === "Delete") {
                        event.preventDefault();
                        closeTab(module);
                      }
                    }}
                  >
                    <Icon size={14} className="shrink-0" aria-hidden="true" />
                    <span className="truncate">{name}</span>
                    {module === "notices" && props.notices.length > 0 && <span className="size-1.5 shrink-0 rounded-full bg-amber-500" aria-hidden="true" />}
                  </TabsTrigger>
                  <Button type="button" variant="ghost" size="icon" className="size-6 shrink-0 hover:text-foreground" aria-label={closeLabel} title={closeLabel} onClick={() => closeTab(module)}>
                    <X size={13} aria-hidden="true" />
                  </Button>
                </div>
              );
            })}
          </TabsList>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button ref={newTabButton} type="button" variant="ghost" size="icon" className="size-7 shrink-0" aria-label={t("Open workspace tab")} title={t("Open workspace tab")}>
                <Plus size={16} aria-hidden="true" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-48" onCloseAutoFocus={(event) => {
              if (openingEditor.current) {
                event.preventDefault();
                openingEditor.current = false;
              }
            }}>
              {workspaceModules.map(({ id, label }) => {
                const Icon = tabIcons[id];
                return <DropdownMenuItem key={id} className="gap-2 text-xs" onSelect={() => openModules([id])}>
                  <Icon size={14} className="text-muted-foreground" aria-hidden="true" />
                  {t(label)}
                </DropdownMenuItem>;
              })}
              <DropdownMenuSeparator />
              <DropdownMenuLabel className="text-xs text-muted-foreground">{t("Tab groups")}</DropdownMenuLabel>
              {groups.map((group) => (
                <DropdownMenuSub key={group.id}>
                  <DropdownMenuSubTrigger className="gap-1 text-xs"><span className="flex-1 truncate">{group.name}</span></DropdownMenuSubTrigger>
                  <DropdownMenuSubContent className="z-50">
                    <DropdownMenuItem className="text-xs" onSelect={() => openModules(group.modules)}>{t("Open group")}</DropdownMenuItem>
                    <DropdownMenuItem className="text-xs" onSelect={() => editGroup(group)}>{t("Edit tab group")}</DropdownMenuItem>
                  </DropdownMenuSubContent>
                </DropdownMenuSub>
              ))}
              <DropdownMenuItem className="text-xs" onSelect={() => editGroup(null)}>{t("New tab group")}</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <Button type="button" variant="ghost" size="icon" className="size-7 shrink-0" aria-label={t("Close workspace panel")} title={t("Close workspace panel")} onClick={onClose}>
            <PanelRightClose size={16} aria-hidden="true" />
          </Button>
        </div>
        {!active && <WorkspaceEmpty>{t("No workspace tabs open")}</WorkspaceEmpty>}
        {open.map((module) => (
          <TabsContent
            key={module}
            value={module}
            forceMount
            className="min-h-0 flex-1 overflow-auto focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring data-[state=inactive]:hidden"
          >
            <ModuleContents {...props} module={module} />
          </TabsContent>
        ))}
      </Tabs>
      {editor && <WorkspaceTabEditor
        tab={editor.group}
        onClose={() => setEditor(undefined)}
        returnFocus={() => newTabButton.current?.focus()}
        onSave={(group) => {
          saveGroup(group);
          openModules(group.modules);
          setEditor(undefined);
        }}
        onDelete={editor.group ? () => {
          if (editor.group) deleteGroup(editor.group.id);
          setEditor(undefined);
        } : undefined}
      />}
    </aside>
  );
}
