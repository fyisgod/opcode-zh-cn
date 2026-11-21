import { t as translate } from "@/lib/i18n";
import { useEffect, useId, useState } from 'react';
import { ChevronRight, FolderOpen, FolderPlus, Loader2, PanelLeft, Plus, SquarePen, Terminal } from 'lucide-react';
import { TabContent } from '@/components/TabContent';
import { TabManager } from '@/components/TabManager';
import { Button } from '@/components/ui/button';
import { useTabContext } from '@/contexts/TabContext';
import { useTabState } from '@/hooks/useTabState';
import { api, type Project, type Session } from '@/lib/api';
import { isTauriRuntime } from '@/lib/apiAdapter';
import { projectName } from '@/lib/projectPaths';
import { cn } from '@/lib/utils';

const RECENT_SESSIONS = 5;
const rowClass = 'min-w-0 rounded-md text-left transition-colors duration-150 motion-reduce:transition-none hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring';

function ProjectGroup({ project, active, sessionId, initiallyExpanded, onSelect, onNewChat, onOpenSession }: {
  project: Project;
  active: boolean;
  sessionId?: string;
  initiallyExpanded: boolean;
  onSelect: () => void;
  onNewChat: () => void;
  onOpenSession: (session: Session) => void;
}) {
  const [expanded, setExpanded] = useState(initiallyExpanded);
  const [showAll, setShowAll] = useState(false);
  const [sessions, setSessions] = useState<Session[] | null>(null);
  const [error, setError] = useState(false);
  const [revision, setRevision] = useState(0);
  const listId = useId();
  const name = projectName(project.path);

  useEffect(() => {
    if (active) setExpanded(true);
  }, [active]);

  useEffect(() => {
    const refresh = (event: Event) => {
      if ((event as CustomEvent<string>).detail === project.path) setRevision(value => value + 1);
    };
    window.addEventListener('project-session-finished', refresh);
    return () => window.removeEventListener('project-session-finished', refresh);
  }, [project.path]);

  useEffect(() => {
    if (!expanded) return;
    let stale = false;
    setError(false);
    api.getProjectSessions(project.id).then(items => {
      if (!stale) setSessions(items);
    }).catch(err => {
      console.error('Failed to load project chats:', err);
      if (!stale) setError(true);
    });
    return () => { stale = true; };
  }, [expanded, project.id, revision]);

  const visibleSessions = showAll ? sessions : sessions?.slice(0, RECENT_SESSIONS);

  return (
    <div className="mb-2">
      <div className={cn('flex h-8 items-center gap-0.5 rounded-md', active && 'bg-accent')}>
        <button
          type="button"
          className={cn(rowClass, 'grid h-8 w-6 shrink-0 place-items-center text-muted-foreground')}
          aria-label={`${expanded ? translate("Collapse") : translate("Expand")} ${name}`}
          aria-expanded={expanded}
          aria-controls={expanded ? listId : undefined}
          onClick={() => setExpanded(value => !value)}
        >
          <ChevronRight className={cn('size-3.5', expanded && 'rotate-90')} />
        </button>
        <button
          type="button"
          className={cn(rowClass, 'flex h-8 flex-1 items-center gap-2 px-1 text-sm')}
          title={project.path}
          onClick={() => { setExpanded(true); onSelect(); }}
        >
          <FolderOpen className="size-4 shrink-0 text-muted-foreground" />
          <span className="truncate">{name}</span>
        </button>
        <button
          type="button"
          className={cn(rowClass, 'grid size-8 shrink-0 place-items-center text-muted-foreground hover:text-foreground')}
          title={translate("New chat in {v1}", { v1: String(name) })}
          aria-label={translate("New chat in {v1}", { v1: String(name) })}
          onClick={() => { setExpanded(true); onNewChat(); }}
        >
          <Plus className="size-4" />
        </button>
      </div>
      {expanded && (
        <div id={listId} className="mt-0.5 space-y-0.5 pl-8 pr-1">
          {visibleSessions?.map(session => {
            const label = session.first_message?.trim().split(/\r?\n/, 1)[0] || translate("Untitled chat");
            const current = active && session.id === sessionId;
            return (
              <button
                key={session.id}
                type="button"
                aria-current={current ? 'true' : undefined}
                title={label}
                onClick={() => onOpenSession(session)}
                className={cn(rowClass, 'block h-8 w-full truncate px-2 text-sm', current ? 'bg-accent text-foreground' : 'text-muted-foreground hover:text-foreground')}
              >
                {label}
              </button>
            );
          })}
          {error ? (
            <button type="button" className={cn(rowClass, 'px-2 py-1.5 text-xs text-destructive')} onClick={() => setRevision(value => value + 1)}>
              {translate("Couldn't load chats. Retry")}
            </button>
          ) : !sessions?.length && (
            <p className="px-2 py-1.5 text-xs text-muted-foreground">{sessions ? translate("No chats yet") : translate("Loading chats...")}</p>
          )}
          {sessions && sessions.length > RECENT_SESSIONS && (
            <button type="button" className={cn(rowClass, 'px-2 py-1.5 text-xs text-muted-foreground hover:text-foreground')} onClick={() => setShowAll(value => !value)}>
              {showAll ? translate("Show fewer") : translate("Show more")}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

export function ProjectWorkspace() {
  const { isReady } = useTabContext();
  const { tabs, activeTab, canAddTab, createChatTab, findTabBySessionId, switchToTab } = useTabState();
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showSidebar, setShowSidebar] = useState(true);
  const [lastProject, setLastProject] = useState<string>();
  const [picking, setPicking] = useState(false);
  const sidebarId = useId();

  const loadProjects = async () => {
    setLoading(true);
    setError(null);
    try {
      setProjects(await api.listProjects());
    } catch (err) {
      console.error('Failed to load projects:', err);
      setError(translate("Could not load projects."));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { void loadProjects(); }, []);

  const activePath = activeTab?.type === 'chat' ? activeTab.initialProjectPath : undefined;
  useEffect(() => {
    if (activePath) setLastProject(activePath);
  }, [activePath]);
  const defaultPath = activePath || lastProject || projects[0]?.path;

  const openDraft = (path: string) => {
    const existing = tabs.find(tab => tab.type === 'chat' && tab.initialProjectPath === path && !tab.sessionId && tab.status === 'idle');
    if (existing) switchToTab(existing.id);
    else if (canAddTab()) createChatTab(undefined, projectName(path), path);
    else setError(translate("Close a tab before opening another chat."));
  };

  useEffect(() => {
    if (!isReady || loading || activeTab) return;
    if (defaultPath && canAddTab()) {
      const draft = tabs.find(tab => tab.type === 'chat' && tab.initialProjectPath === defaultPath && !tab.sessionId);
      if (draft) switchToTab(draft.id);
      else createChatTab(undefined, projectName(defaultPath), defaultPath);
    } else if (tabs.length) {
      switchToTab(tabs[0].id);
    }
  }, [isReady, loading, activeTab, defaultPath, tabs, canAddTab, switchToTab, createChatTab]);

  const addProject = async (path: string) => {
    if (!path.trim()) return;
    setError(null);
    const known = projects.find(project => project.path === path);
    if (known) {
      openDraft(known.path);
      return;
    }

    const project = await api.createProject(path);
    // Listing again also preserves the backend's canonical project path and session order.
    const listed = await api.listProjects().catch(() => null);
    setProjects(previous => listed ?? [project, ...previous.filter(item => item.id !== project.id)]);
    openDraft(listed?.find(item => item.id === project.id)?.path ?? project.path);
  };

  const selectFolder = async () => {
    if (!isTauriRuntime()) {
      setError(translate("Open project folders in the desktop app."));
      return;
    }
    setPicking(true);
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({ directory: true, multiple: false, title: translate("Open project folder") });
      if (typeof selected === 'string') await addProject(selected);
    } catch (err) {
      console.error('Could not open project folder:', err);
      setError(translate("Could not open the project folder."));
    } finally {
      setPicking(false);
    }
  };

  const newChat = () => {
    if (defaultPath) openDraft(defaultPath);
    else void selectFolder();
  };

  const openSession = (session: Session) => {
    const existing = findTabBySessionId(session.id);
    if (existing) switchToTab(existing.id);
    else if (canAddTab()) {
      const title = session.first_message?.trim().split(/\r?\n/, 1)[0].slice(0, 100) || projectName(session.project_path);
      createChatTab(session.id, title, session.project_path, session);
    } else setError(translate("Close a tab before opening another chat."));
  };

  const selectProject = (path: string) => {
    if (activePath === path) return;
    const existing = tabs.find(tab => tab.type === 'chat' && tab.initialProjectPath === path);
    if (existing) switchToTab(existing.id);
    else openDraft(path);
  };

  return (
    <div className="flex h-full min-h-0 bg-background">
      <aside id={sidebarId} aria-label={translate("Projects and chats")} className={cn('w-[clamp(176px,27vw,290px)] shrink-0 flex-col border-r border-border/70 bg-card/70', showSidebar ? 'flex' : 'hidden')}>
        <div className="space-y-4 px-3 pt-3">
          <button type="button" onClick={newChat} disabled={!isReady || loading || picking} className={cn(rowClass, 'flex h-9 w-full items-center gap-2 px-2.5 text-sm font-medium disabled:opacity-50')}>
            <SquarePen className="size-4 text-muted-foreground" />
            {translate("New chat")}
          </button>
          <div className="flex items-center justify-between pl-2.5">
            <h2 className="text-xs font-semibold text-muted-foreground">{translate("Projects")}</h2>
            <Button variant="ghost" size="icon" className="size-8" title={translate("Open project folder")} aria-label={translate("Open project folder")} disabled={!isReady || loading || picking} onClick={() => void selectFolder()}>
              <FolderPlus className="size-4" />
            </Button>
          </div>
        </div>
        <nav aria-label={translate("Project list")} className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-3 pb-3">
          {projects.map((project, index) => (
            <ProjectGroup
              key={project.id}
              project={project}
              active={activePath === project.path}
              sessionId={activeTab?.sessionId}
              initiallyExpanded={index < 5}
              onSelect={() => selectProject(project.path)}
              onNewChat={() => openDraft(project.path)}
              onOpenSession={openSession}
            />
          ))}
          {loading && <p className="px-2.5 py-2 text-xs text-muted-foreground" role="status">{translate("Loading projects...")}</p>}
          {error && (projects.length > 0 || activeTab) && <p className="px-2.5 py-2 text-xs text-destructive" role="alert">{error}</p>}
          {!loading && error && !projects.length && (
            <button type="button" onClick={() => void loadProjects()} className={cn(rowClass, 'px-2.5 py-2 text-xs text-muted-foreground')}>{translate("Retry project list")}</button>
          )}
        </nav>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-9 shrink-0 items-stretch border-b border-border/50">
          <Button variant="ghost" size="icon" onClick={() => setShowSidebar(value => !value)} aria-controls={sidebarId} aria-expanded={showSidebar} title={showSidebar ? translate("Hide sidebar") : translate("Show sidebar")} aria-label={showSidebar ? translate("Hide sidebar") : translate("Show sidebar")} className="m-0.5 size-8 shrink-0 text-muted-foreground">
            <PanelLeft className="size-4" />
          </Button>
          <TabManager onNewChat={newChat} className="min-w-0 flex-1 border-b-0 bg-transparent" />
        </div>
        <div className="relative min-h-0 flex-1 overflow-hidden">
          <TabContent emptyState={
            <div className="flex h-full flex-col items-center justify-center gap-5 p-5 text-center">
              {loading || !isReady ? <Loader2 className="size-5 animate-spin text-muted-foreground motion-reduce:animate-none" aria-label={translate("Loading workspace")} /> : (
                <>
                  <Terminal className="size-9 text-muted-foreground" aria-hidden="true" />
                  <h1 className="text-xl font-medium">{translate("Open a project to start a chat")}</h1>
                  <Button onClick={() => void selectFolder()} disabled={picking}><FolderPlus className="size-4" /> {translate("Open folder")}</Button>
                  {error && <p className="text-xs text-destructive" role="alert">{error}</p>}
                </>
              )}
            </div>
          } />
        </div>
      </main>

    </div>
  );
}
