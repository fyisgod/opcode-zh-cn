import { useRef, useState } from "react";
import { ShieldAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { type WorkspaceApproval } from "@/hooks/useWorkspaceApprovals";
import { t } from "@/lib/i18n";

export function WorkspaceApprovalDialog({ request, onDecide }: {
  request: WorkspaceApproval;
  onDecide: (id: string, allow: boolean) => Promise<void>;
}) {
  const denyButton = useRef<HTMLButtonElement>(null);
  const responding = useRef(false);
  const [busy, setBusy] = useState(false);
  const read = ["Read", "Glob", "Grep"].includes(request.tool);
  const decide = (allow: boolean) => {
    if (responding.current) return;
    responding.current = true;
    setBusy(true);
    void onDecide(request.requestId, allow);
  };

  return (
    <Dialog open onOpenChange={open => { if (!open) decide(false); }}>
      <DialogContent
        className="w-[calc(100%-2rem)] rounded-lg motion-reduce:animate-none"
        onPointerDownOutside={event => event.preventDefault()}
        onOpenAutoFocus={event => { event.preventDefault(); denyButton.current?.focus(); }}
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 pr-6 text-base">
            <ShieldAlert aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
            {t(read ? "Allow a read outside the workspace?" : "Allow a change outside the workspace?")}
          </DialogTitle>
          <DialogDescription>{t("This approval applies only to this operation.")}</DialogDescription>
        </DialogHeader>
        <dl className="grid gap-2 rounded-md border border-border bg-muted/40 p-3 text-xs">
          <div>
            <dt className="mb-1 text-muted-foreground">{t("Path")}</dt>
            <dd className="max-h-24 overflow-auto break-all font-mono" dir="ltr">{request.resolvedPath ?? request.path}</dd>
          </div>
          {request.resolvedPath && request.resolvedPath !== request.path && (
            <div>
              <dt className="mb-1 text-muted-foreground">{t("Requested path")}</dt>
              <dd className="max-h-16 overflow-auto break-all font-mono" dir="ltr">{request.path}</dd>
            </div>
          )}
          {request.pattern && (
            <div>
              <dt className="mb-1 text-muted-foreground">{t("Search pattern")}</dt>
              <dd className="max-h-16 overflow-auto break-all font-mono" dir="ltr">{request.pattern}</dd>
            </div>
          )}
        </dl>
        <DialogFooter className="gap-2 sm:space-x-0">
          <Button ref={denyButton} variant="outline" disabled={busy} onClick={() => decide(false)}>{t("Deny")}</Button>
          <Button disabled={busy} onClick={() => decide(true)}>{t("Allow once")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
