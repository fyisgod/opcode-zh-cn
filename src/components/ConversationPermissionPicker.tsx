import { ShieldAlert, ShieldCheck, ShieldPlus } from "lucide-react";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { type ConversationPermission } from "@/lib/conversationPermissions";
import { t } from "@/lib/i18n";
import { isTauriRuntime } from "@/lib/apiAdapter";

const MODES = {
  readOnly: {
    icon: ShieldCheck,
    label: "Read-only",
    description: "Read files in this workspace without editing them.",
  },
  workspaceWrite: {
    icon: ShieldPlus,
    label: "Workspace write",
    description: "Read and edit workspace files. Cannot run commands.",
  },
  fullAccess: {
    icon: ShieldAlert,
    label: "Full access",
    description: "Run commands and access files outside this workspace without asking.",
  },
} as const;

const modeIds = Object.keys(MODES) as ConversationPermission[];

export function ConversationPermissionPicker({ value, onChange, disabled }: {
  value: ConversationPermission;
  onChange: (value: ConversationPermission) => void;
  disabled: boolean;
}) {
  const selected = MODES[value];
  const describe = (id: ConversationPermission) => t(id === "workspaceWrite" && isTauriRuntime()
    ? "Outside access requires approval. Commands are disabled."
    : MODES[id].description);
  return (
    <Select value={value} onValueChange={id => {
      const next = modeIds.find(mode => mode === id);
      if (next) onChange(next);
    }} disabled={disabled}>
      <SelectTrigger
        aria-label={t("Conversation permissions")}
        title={describe(value)}
        className="h-9 w-auto gap-1.5 rounded-md border-transparent bg-transparent px-2 text-xs text-muted-foreground shadow-none transition-colors duration-150 hover:bg-accent hover:text-foreground [&>svg]:size-3"
      >
        <SelectValue>
          <span className="flex items-center gap-1.5 whitespace-nowrap">
            <selected.icon aria-hidden="true" className="size-4 shrink-0" />
            {t(selected.label)}
          </span>
        </SelectValue>
      </SelectTrigger>
      <SelectContent side="top" align="start" className="w-[min(18rem,calc(100vw-1.5rem))]">
        {modeIds.map(id => {
          const { icon: Icon, label } = MODES[id];
          return (
            <SelectItem key={id} value={id} textValue={t(label)} className="py-1.5">
              <span className="flex items-start gap-2">
                <Icon aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                <span className="grid gap-0.5">
                  <span className="font-medium">{t(label)}</span>
                  <span className="text-xs text-muted-foreground">{describe(id)}</span>
                </span>
              </span>
            </SelectItem>
          );
        })}
      </SelectContent>
    </Select>
  );
}
