import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "@/lib/api";
import { listen, type UnlistenFn } from "@/lib/tauriEvents";

export interface WorkspaceApproval {
  requestId: string;
  tool: string;
  path: string;
  resolvedPath: string | null;
  pattern: string | null;
}

type ApprovalEvent = ({ type: "requested" } & WorkspaceApproval) | { type: "dismissed"; requestId: string };

type TurnListener = { id: string; unlisten?: UnlistenFn };

export function useWorkspaceApprovals() {
  const [requests, setRequests] = useState<WorkspaceApproval[]>([]);
  const current = useRef<TurnListener>();
  const stop = useCallback(() => {
    const turn = current.current;
    current.current = undefined;
    if (!turn) return;
    turn.unlisten?.();
    void api.closeClaudeApprovalTurn(turn.id).catch(console.error);
  }, []);

  useEffect(() => stop, [stop]);

  const close = () => {
    stop();
    setRequests([]);
  };

  const open = async () => {
    close();
    const turn: TurnListener = { id: crypto.randomUUID() };
    current.current = turn;
    const unlisten = await listen<ApprovalEvent>(`claude-approval:${turn.id}`, ({ payload }) => {
      if (current.current !== turn) return;
      setRequests(prev => payload.type === "requested"
        ? [...prev.filter(item => item.requestId !== payload.requestId), payload]
        : prev.filter(item => item.requestId !== payload.requestId));
    });
    if (current.current !== turn) {
      unlisten();
      return undefined;
    }
    turn.unlisten = unlisten;
    return turn.id;
  };

  const finishOpening = (id: string | undefined) => {
    // The tab may have closed while the backend was still launching the process.
    if (id && current.current?.id !== id) void api.closeClaudeApprovalTurn(id).catch(console.error);
  };

  const decide = async (requestId: string, allow: boolean) => {
    try {
      await api.decideClaudeApproval(requestId, allow);
    } finally {
      setRequests(prev => prev.filter(item => item.requestId !== requestId));
    }
  };

  return { request: requests[0], open, close, finishOpening, decide };
}
