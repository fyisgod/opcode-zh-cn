import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, File, Folder, GitBranch, RefreshCw } from "lucide-react";
import { isTauriRuntime } from "@/lib/apiAdapter";
import { t } from "@/lib/i18n";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

export type WorkspaceBrowserProps = { root: string; revision: number };

type Entry = { name: string; directory: boolean; target: string | null };
type Preview = { text: string; binary: boolean; truncated: boolean };
type Change = { path: string; index: string; worktree: string };
type Status = { root: string; branch: string; unborn: boolean; changes: Change[] };
type Commit = { id: string; short: string; author: string; date: string; subject: string };
type History = { commits: Commit[]; more: boolean };

type Request =
  | { kind: "files" | "file"; path: string }
  | { kind: "status" }
  | { kind: "tree" | "blob" | "commit"; object: string }
  | { kind: "history"; offset: number }
  | { kind: "diff"; path: string; staged: boolean; untracked: boolean };

type Query<T> = { data?: T; error?: string };

function useWorkspaceQuery<T>(root: string, request: Request | null, revision: number): Query<T> {
  const key = request === null ? "" : JSON.stringify(request);
  const [result, setResult] = useState<Query<T> & { root: string; key: string }>();
  useEffect(() => {
    if (!root || !key || !isTauriRuntime()) return;
    let active = true;
    invoke<T>("inspect_workspace", { root, request: JSON.parse(key) }).then(
      (data) => { if (active) setResult({ root, key, data }); },
      (error: unknown) => { if (active) setResult({ root, key, error: String(error) }); },
    );
    return () => { active = false; };
  }, [root, key, revision]);
  return result?.root === root && result.key === key ? result : {};
}

export function WorkspaceEmpty({ children }: { children: React.ReactNode }) {
  return <p className="px-3 py-5 text-xs leading-relaxed text-muted-foreground">{children}</p>;
}

function LoadError({ error }: { error?: string }) {
  return error ? (
    <p role="status" className="break-words px-3 py-4 text-xs text-destructive">{t(error)}</p>
  ) : <WorkspaceEmpty>{t("Loading…")}</WorkspaceEmpty>;
}

const rowClass = "flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-2 text-left text-xs transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring disabled:cursor-default disabled:opacity-50 disabled:hover:bg-transparent";

function TextPreview({ query, diff = false }: { query: Query<Preview>; diff?: boolean }) {
  if (!query.data) return <LoadError error={query.error} />;
  const { text, binary, truncated } = query.data;
  if (binary || !text) {
    return <WorkspaceEmpty>{t(binary ? "Binary file preview is unavailable" : diff ? "No diff to display" : "This file is empty")}</WorkspaceEmpty>;
  }
  return (
    <>
      {truncated && <p className="border-b border-border px-3 py-2 text-xs text-muted-foreground">{t("Preview limited to 128 KiB")}</p>}
      <pre tabIndex={0} className="overflow-x-auto px-3 py-2 font-mono text-xs leading-relaxed text-foreground">
        {diff ? text.split("\n").map((line, index) => (
          <span key={index} className={cn("block min-w-max pr-3",
            line.startsWith("+") && !line.startsWith("+++") && "bg-green-500/10 text-green-700 dark:text-green-400",
            line.startsWith("-") && !line.startsWith("---") && "bg-destructive/10 text-destructive",
            line.startsWith("@@") && "text-primary",
          )}>{line || "\u00A0"}</span>
        )) : text}
      </pre>
    </>
  );
}

type Location = { label: string; target: string };

function DirectoryBrowser({ root, revision, tree }: WorkspaceBrowserProps & { tree?: Location }) {
  const [folders, setFolders] = useState<Location[]>([]);
  const [selected, setSelected] = useState<Location>();
  const location = folders.at(-1) ?? tree ?? { label: t("Workspace files"), target: "" };
  const listing = useWorkspaceQuery<Entry[]>(root, selected ? null : tree
    ? { kind: "tree", object: location.target }
    : { kind: "files", path: location.target }, revision);
  const preview = useWorkspaceQuery<Preview>(root, !selected ? null : tree
    ? { kind: "blob", object: selected.target }
    : { kind: "file", path: selected.target }, revision);
  const crumbs = [tree ?? { label: t("Workspace files"), target: "" }, ...folders];

  return (
    <>
      <nav aria-label={t("File path")} className="flex min-w-0 flex-wrap items-center gap-1 border-b border-border px-3 py-2 text-xs text-muted-foreground">
        {crumbs.map(({ label }, index) => (
          <div className="flex min-w-0 items-center gap-1" key={index}>
            {index > 0 && <ChevronRight size={12} className="shrink-0" />}
            <button type="button" className="truncate rounded px-1 py-0.5 hover:bg-accent hover:text-foreground" title={label} onClick={() => {
              setFolders(folders.slice(0, index));
              setSelected(undefined);
            }}>{label}</button>
          </div>
        ))}
        {selected && <span title={selected.label} className="min-w-0 truncate font-medium text-foreground">/ {selected.label}</span>}
      </nav>
      {selected ? <TextPreview query={preview} /> : !listing.data ? <LoadError error={listing.error} /> : !listing.data.length ? (
        <WorkspaceEmpty>{t("This folder is empty")}</WorkspaceEmpty>
      ) : (
        <ul className="space-y-0.5 p-2">
          {listing.data.map((entry) => (
            <li key={entry.name}>
              <button type="button" className={rowClass} disabled={entry.target === null} title={entry.name} onClick={() => {
                if (entry.target === null) return;
                const next = { label: entry.name, target: entry.target };
                if (entry.directory) setFolders([...folders, next]);
                else setSelected(next);
              }}>
                {entry.directory ? <Folder size={15} className="shrink-0 text-muted-foreground" /> : <File size={15} className="shrink-0 text-muted-foreground" />}
                <span className="truncate">{entry.name}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

function BrowserHeader({ label, onRefresh }: { label: string; onRefresh: () => void }) {
  return (
    <div className="flex min-h-10 items-center justify-between gap-2 border-b border-border px-3">
      <span className="min-w-0 truncate text-xs font-medium" title={label}>{label}</span>
      <Button variant="ghost" size="icon" className="size-7 shrink-0" title={t("Refresh")} aria-label={t("Refresh")} onClick={onRefresh}>
        <RefreshCw size={14} />
      </Button>
    </div>
  );
}

export function WorkspaceFiles({ root, revision }: WorkspaceBrowserProps) {
  const [refresh, setRefresh] = useState(0);
  if (!root) return <WorkspaceEmpty>{t("No project selected")}</WorkspaceEmpty>;
  if (!isTauriRuntime()) return <WorkspaceEmpty>{t("Available in the Opcode desktop app")}</WorkspaceEmpty>;
  return (
    <>
      <BrowserHeader label={root} onRefresh={() => setRefresh((value) => value + 1)} />
      <DirectoryBrowser root={root} revision={revision + refresh} />
    </>
  );
}

function ChangeList({ root, changes, revision }: WorkspaceBrowserProps & { changes: Change[] }) {
  const [selection, setSelection] = useState<{ path: string; staged: boolean; untracked: boolean }>();
  const query = useWorkspaceQuery<Preview>(root, selection ? { kind: "diff", ...selection } : null, revision);
  if (selection) return (
    <>
      <button type="button" className={cn(rowClass, "rounded-none border-b border-border px-3")} onClick={() => setSelection(undefined)}>
        <ChevronLeft size={15} className="shrink-0" />
        <span className="truncate" title={selection.path}>{selection.path}</span>
        <span className="ml-auto shrink-0 text-muted-foreground">{t(selection.staged ? "Staged" : selection.untracked ? "Untracked" : "Unstaged")}</span>
      </button>
      <TextPreview query={query} diff />
    </>
  );
  if (!changes.length) return <WorkspaceEmpty>{t("Working tree is clean")}</WorkspaceEmpty>;
  const groups = [
    { label: "Staged", staged: true, rows: changes.filter((change) => ![" ", "?", "U"].includes(change.index) && change.worktree !== "U" && !/^(DD|AA)$/.test(change.index + change.worktree)) },
    { label: "Unstaged", staged: false, rows: changes.filter((change) => change.worktree !== " " && change.index !== "?") },
    { label: "Untracked", staged: false, rows: changes.filter((change) => change.index === "?") },
  ];
  return <div className="space-y-3 p-2">{groups.filter(({ rows }) => rows.length > 0).map(({ label, staged, rows }) => (
    <section key={label}>
      <h3 className="flex justify-between px-2 py-1 text-xs font-medium text-muted-foreground">{t(label)} <span>{rows.length}</span></h3>
      <ul>{rows.map((change) => (
        <li key={change.path}>
          <button type="button" className={rowClass} title={change.path} onClick={() => setSelection({ path: change.path, staged, untracked: change.index === "?" })}>
            <File size={14} className="shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate" dir="auto">{change.path}</span>
            <span className="shrink-0 font-mono text-primary">{staged ? change.index : change.worktree}</span>
          </button>
        </li>
      ))}</ul>
    </section>
  ))}</div>;
}

const HISTORY_PAGE = 40;

function CommitHistory({ root, revision, onTree }: WorkspaceBrowserProps & { onTree: (commit: Commit) => void }) {
  const [offset, setOffset] = useState(0);
  const [selected, setSelected] = useState<Commit>();
  const history = useWorkspaceQuery<History>(root, selected ? null : { kind: "history", offset }, revision);
  const diff = useWorkspaceQuery<Preview>(root, selected ? { kind: "commit", object: selected.id } : null, revision);
  if (selected) return (
    <>
      <div className="flex min-w-0 items-center justify-between gap-1 border-b border-border p-2">
        <button type="button" className={rowClass} onClick={() => setSelected(undefined)}>
          <ChevronLeft size={14} className="shrink-0" />
          <span className="truncate" title={selected.subject}>{selected.subject || selected.short}</span>
        </button>
        <Button variant="ghost" size="sm" className="h-7 shrink-0 text-xs" onClick={() => onTree(selected)}>{t("View tree")}</Button>
      </div>
      <TextPreview query={diff} diff />
    </>
  );
  if (!history.data) return <LoadError error={history.error} />;
  return (
    <>
      {!history.data.commits.length && <WorkspaceEmpty>{t("No commits found")}</WorkspaceEmpty>}
      <ul className="divide-y divide-border/70 px-2">
        {history.data.commits.map((commit) => (
          <li key={commit.id}>
            <button type="button" className={cn(rowClass, "flex-col items-start gap-1 py-2.5")} onClick={() => setSelected(commit)}>
              <span className="w-full truncate font-medium" title={commit.subject}>{commit.subject || t("No commit message")}</span>
              <span className="flex w-full items-center gap-2 text-muted-foreground">
                <code className="shrink-0 text-primary" title={commit.id}>{commit.short}</code>
                <span className="min-w-0 flex-1 truncate">{commit.author}</span>
                <time className="shrink-0" dateTime={commit.date}>{commit.date.slice(0, 10)}</time>
              </span>
            </button>
          </li>
        ))}
      </ul>
      {(offset > 0 || history.data.more) && (
        <div className="flex justify-between px-3 py-2">
          <Button variant="ghost" size="sm" className="h-7 text-xs" disabled={offset === 0} onClick={() => setOffset(offset - HISTORY_PAGE)}>{t("Newer commits")}</Button>
          <Button variant="ghost" size="sm" className="h-7 text-xs" disabled={!history.data.more} onClick={() => setOffset(offset + HISTORY_PAGE)}>{t("Older commits")}</Button>
        </div>
      )}
    </>
  );
}

const gitViews = ["Changes", "History", "Tree"] as const;

export function WorkspaceGit({ root, revision }: WorkspaceBrowserProps) {
  const [refresh, setRefresh] = useState(0);
  const [view, setView] = useState<typeof gitViews[number]>("Changes");
  const [commit, setCommit] = useState<Commit>();
  const status = useWorkspaceQuery<Status>(root, { kind: "status" }, revision + refresh);
  if (!root) return <WorkspaceEmpty>{t("No project selected")}</WorkspaceEmpty>;
  if (!isTauriRuntime()) return <WorkspaceEmpty>{t("Available in the Opcode desktop app")}</WorkspaceEmpty>;
  const repository = status.data;
  return (
    <>
      <BrowserHeader label={repository?.root ?? root} onRefresh={() => setRefresh((value) => value + 1)} />
      {repository ? (
        <>
          <div className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground" title={repository.branch}>
            <GitBranch size={15} className="shrink-0" />
            <span className="truncate font-medium text-foreground">{repository.branch}</span>
          </div>
          <div className="flex gap-1 border-b border-border px-2 pb-2" role="group" aria-label={t("Git views")}>
            {gitViews.map((item) => (
              <Button key={item} variant={view === item ? "secondary" : "ghost"} size="sm" className="h-7 flex-1 text-xs" aria-pressed={view === item} onClick={() => setView(item)}>{t(item)}</Button>
            ))}
          </div>
          {view === "Changes" && <ChangeList root={root} revision={revision + refresh} changes={repository.changes} />}
          {view !== "Changes" && repository.unborn && <WorkspaceEmpty>{t("No commits found")}</WorkspaceEmpty>}
          {view === "History" && !repository.unborn && <CommitHistory root={root} revision={revision + refresh} onTree={(next) => { setCommit(next); setView("Tree"); }} />}
          {view === "Tree" && !repository.unborn && (
            <>
              {commit && <div className="flex items-center justify-between gap-2 px-3 py-1.5 text-xs">
                <code title={commit.id}>{commit.short}</code>
                <Button variant="ghost" size="sm" className="h-7 text-xs" onClick={() => setCommit(undefined)}>{t("Back to HEAD")}</Button>
              </div>}
              <DirectoryBrowser key={commit?.id ?? "HEAD"} root={root} revision={revision + refresh} tree={{ target: commit?.id ?? "HEAD", label: commit?.short ?? "HEAD" }} />
            </>
          )}
        </>
      ) : <LoadError error={status.error} />}
    </>
  );
}
