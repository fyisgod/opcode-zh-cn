export type ConversationPermission = "readOnly" | "workspaceWrite" | "fullAccess";

export const DEFAULT_CONVERSATION_PERMISSION: ConversationPermission = "workspaceWrite";

const storageKey = (sessionId: string) => `opcode.conversation-permission.${sessionId}`;

export function loadConversationPermission(sessionId?: string): ConversationPermission {
  try {
    const value = sessionId && localStorage.getItem(storageKey(sessionId));
    if (value === "readOnly" || value === "workspaceWrite" || value === "fullAccess") return value;
  } catch {
    // Use the default if storage is unavailable.
  }
  return DEFAULT_CONVERSATION_PERMISSION;
}

export function saveConversationPermission(sessionId: string, value: ConversationPermission) {
  try {
    localStorage.setItem(storageKey(sessionId), value);
  } catch {
    // The current tab still uses the selected permission.
  }
}
