import type { ClaudeStreamMessage } from "../AgentExecution";

export type NoticeLevel = "Warning" | "Error" | "Info";
export type WorkspaceNotice = { level: NoticeLevel; text: string; time?: number };
export type RunState = "Running" | "Completed" | "Failed" | "Stopped" | "Pending";

type Activity = {
  id: string;
  kind: "agent" | "task";
  title: string;
  state: RunState;
  background?: boolean;
  parent?: string;
  taskId?: string;
  agentType?: string;
  details?: string;
};

export function noticeLevel(text: string): NoticeLevel {
  if (/\b(warning|warn)\b|permission mode forced|⚠/i.test(text)) return "Warning";
  if (/\b(error|failed|fatal|exception)\b/i.test(text)) return "Error";
  return "Info";
}

export function activityToolKind(part: { name?: string; input?: Record<string, unknown> }) {
  const name = part.name?.toLowerCase();
  if (name === "agent" || name === "task") return "agent";
  if (name === "monitor" || (name === "bash" && part.input?.run_in_background === true)) return "task";
  return undefined;
}

function resultText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content.flatMap((part) => typeof part?.text === "string" ? [part.text] : []).join("\n");
}

export function readSessionWorkspace(messages: ClaudeStreamMessage[]) {
  let init: ClaudeStreamMessage | undefined;
  let result: ClaudeStreamMessage | undefined;
  const notices: WorkspaceNotice[] = [];
  const activity = new Map<string, Activity>();
  const taskLinks = new Map<string, string>();
  const groupedTools = new Set<string>();

  for (const message of messages) {
    const { type, subtype } = message;
    if (type === "system") {
      if (subtype === "init") init = message;
      if (subtype === "warning" || subtype === "error" || subtype === "notice" || subtype === "info") {
        const text = [message.message, message.text, message.error, message.warning, message.result, message.message?.content]
          .map(resultText).find(Boolean);
        if (text) notices.push({ level: subtype === "error" ? "Error" : subtype === "warning" ? "Warning" : "Info", text });
      }
      if (subtype === "task_started" || subtype === "task_progress" || subtype === "task_notification") {
        const taskId = message.task_id;
        const id = message.tool_use_id || taskLinks.get(taskId) || taskId;
        if (typeof id !== "string") continue;
        if (typeof taskId === "string") {
          const previousId = taskLinks.get(taskId) || taskId;
          if (previousId !== id && activity.has(previousId)) {
            const previous = activity.get(previousId)!;
            activity.set(id, { ...previous, ...activity.get(id), id });
            activity.delete(previousId);
          }
          taskLinks.set(taskId, id);
        }
        const previous = activity.get(id);
        const status = message.status;
        const state = status === "completed" ? "Completed" : status === "failed" ? "Failed" :
          status === "stopped" || status === "cancelled" || status === "canceled" ? "Stopped" :
            subtype === "task_notification" ? previous?.state ?? "Pending" : "Running";
        activity.set(id, {
          id,
          kind: previous?.kind ?? (message.task_type === "local_agent" || message.task_type === "agent" ? "agent" : "task"),
          title: message.description || previous?.title || taskId || id,
          agentType: previous?.agentType,
          parent: previous?.parent,
          taskId: typeof taskId === "string" ? taskId : previous?.taskId,
          background: true,
          state,
          details: message.summary || previous?.details,
        });
      }
    }
    if (type === "result" && !message.parent_tool_use_id) {
      result = message;
      if (message.is_error || (subtype && subtype !== "success")) {
        const text = resultText(message.result) || resultText(message.error) || (Array.isArray(message.errors) ? message.errors.filter((error: unknown) => typeof error === "string").join("\n") : "") || subtype;
        if (text) notices.push({ level: "Error", text });
      }
    }
    if (message.parent_tool_use_id && type === "assistant") {
      const parent = activity.get(message.parent_tool_use_id);
      const details = resultText(message.message?.content);
      if (parent?.kind === "agent" && details) activity.set(parent.id, { ...parent, details });
    }
    if (!Array.isArray(message.message?.content)) continue;
    for (const part of message.message.content) {
      if (part.type === "tool_use" && typeof part.id === "string") {
        const kind = activityToolKind(part);
        if (!kind) continue;
        const previous = activity.get(part.id);
        groupedTools.add(part.id);
        activity.set(part.id, {
          id: part.id,
          kind,
          title: part.input?.description || part.input?.name || part.name,
          parent: message.parent_tool_use_id,
          taskId: previous?.taskId,
          agentType: part.input?.subagent_type,
          background: previous?.background || kind === "task" || part.input?.run_in_background === true,
          state: previous?.state ?? "Running",
          details: previous?.details,
        });
      }
      if (part.type === "tool_result" && typeof part.tool_use_id === "string") {
        const previous = activity.get(part.tool_use_id);
        if (!previous) continue;
        const details = resultText(part.content);
        // Older CLI versions report the background task ID in the tool result.
        const taskId = details.match(/(?:background (?:task )?ID[: ]+|<task-id>)([\w-]+)/i)?.[1];
        if (taskId) {
          const priorNotification = activity.get(taskId);
          taskLinks.set(taskId, previous.id);
          if (priorNotification && priorNotification.id !== previous.id) activity.delete(taskId);
          activity.set(previous.id, {
            ...previous, taskId, background: true, state: priorNotification?.state ?? previous.state,
            details: priorNotification?.details || details,
          });
        } else if (part.is_error || !previous.background) {
          activity.set(previous.id, { ...previous, state: part.is_error ? "Failed" : "Completed", details: details || previous.details });
        } else if (!previous.details && details) {
          activity.set(previous.id, { ...previous, details });
        }
      }
    }
  }
  return { init, result, notices, groupedTools, activity: [...activity.values()].reverse() };
}
