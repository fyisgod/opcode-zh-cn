import { useEffect, useId, useState, type FormEvent } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { t } from "@/lib/i18n";
import { reasoningEfforts, reasoningEffortLabels, type ProviderModel, type ReasoningEffort } from "@/lib/modelProvider";

interface ModelEditorProps {
  model?: ProviderModel;
  existingIds: Set<string>;
  onClose: () => void;
  onApply: (model: ProviderModel) => void;
}

export function ModelEditor({ model, existingIds, onClose, onApply }: ModelEditorProps) {
  const fieldId = useId();
  const [id, setId] = useState(model?.id ?? "");
  const [displayName, setDisplayName] = useState(model?.displayName ?? "");
  const [contextLength, setContextLength] = useState(model?.contextLength?.toString() ?? "");
  const [efforts, setEfforts] = useState(model?.reasoningEfforts ?? []);
  const [error, setError] = useState<string>();

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const modelId = id.trim();
    if (!modelId || modelId.startsWith("-") || /[\s\p{Cc}]/u.test(modelId)) {
      setError("Enter a valid model ID.");
      return;
    }
    if (existingIds.has(modelId) && modelId !== model?.id) {
      setError("This model ID is already configured.");
      return;
    }
    if (/\p{Cc}/u.test(displayName.trim())) {
      setError("Enter a valid model display name.");
      return;
    }
    const tokens = Number(contextLength);
    if (contextLength && (!/^\d+$/.test(contextLength) || tokens < 1 || tokens > 0xFFFFFFFF)) {
      setError("Context length must be a positive whole number.");
      return;
    }
    onApply({
      id: modelId,
      displayName: displayName.trim() || modelId,
      contextLength: contextLength ? tokens : undefined,
      reasoningEfforts: reasoningEfforts.filter(effort => efforts.includes(effort)),
    });
  };

  const toggleEffort = (effort: ReasoningEffort) => setEfforts(current => current.includes(effort)
    ? current.filter(value => value !== effort) : [...current, effort]);

  return (
    <Dialog open onOpenChange={open => { if (!open) onClose(); }}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] overflow-y-auto rounded-lg" aria-describedby={undefined}>
        <DialogHeader><DialogTitle>{t(model ? "Edit model" : "Manually add model")}</DialogTitle></DialogHeader>
        <form onSubmit={submit} onChange={() => setError(undefined)} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor={`${fieldId}-id`}>{t("Model ID")}</Label>
            <Input id={`${fieldId}-id`} value={id} spellCheck={false} onChange={event => setId(event.target.value)} />
          </div>
          <div className="space-y-2">
            <Label htmlFor={`${fieldId}-name`}>{t("Display name")}</Label>
            <Input id={`${fieldId}-name`} value={displayName} placeholder={id.trim() || t("Defaults to model ID")}
              onChange={event => setDisplayName(event.target.value)} />
          </div>
          <div className="space-y-2">
            <Label htmlFor={`${fieldId}-context`}>{t("Context length in tokens (optional)")}</Label>
            <Input id={`${fieldId}-context`} value={contextLength} inputMode="numeric" placeholder="200000"
              onChange={event => setContextLength(event.target.value)} />
          </div>
          <fieldset className="space-y-2">
            <legend className="text-sm font-medium">{t("Supported thinking levels (optional)")}</legend>
            <div className="grid grid-cols-3 gap-2 sm:grid-cols-4">
              {reasoningEfforts.map(effort => (
                <label key={effort} className="flex min-h-8 cursor-pointer items-center gap-2 rounded-md border px-2.5 text-xs transition-colors hover:bg-accent has-checked:bg-accent">
                  <input type="checkbox" className="size-4 accent-primary" checked={efforts.includes(effort)} onChange={() => toggleEffort(effort)} />
                  {t(reasoningEffortLabels[effort])}
                </label>
              ))}
            </div>
          </fieldset>
          {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
          <DialogFooter className="gap-2 sm:space-x-0">
            <Button type="button" variant="outline" onClick={onClose}>{t("Cancel")}</Button>
            <Button type="submit">{t(model ? "Apply" : "Add model")}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

interface ModelCatalogProps {
  request: Promise<ProviderModel[]>;
  existingIds: Set<string>;
  onRetry: () => void;
  onClose: () => void;
  onAdd: (models: ProviderModel[]) => void;
}

export function ModelCatalog({ request, existingIds, onRetry, onClose, onAdd }: ModelCatalogProps) {
  const [result, setResult] = useState<{ models: ProviderModel[] } | { error: string }>();
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(() => new Set());

  useEffect(() => {
    let active = true;
    setResult(undefined);
    void request.then(models => {
      if (active) setResult({ models });
    }, error => {
      if (active) setResult({ error: typeof error === "string" ? error : "Could not fetch the model list." });
    });
    return () => { active = false; };
  }, [request]);

  const models = result && "models" in result ? result.models : [];
  const search = query.trim().toLowerCase();
  const filtered = models.filter(model => model.id.toLowerCase().includes(search) || model.displayName.toLowerCase().includes(search));
  const toggle = (id: string) => setSelected(previous => {
    const next = new Set(previous);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  });

  return (
    <Dialog open onOpenChange={open => { if (!open) onClose(); }}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] overflow-y-auto rounded-lg">
        <DialogHeader>
          <DialogTitle>{t("Available models")}</DialogTitle>
          <DialogDescription>{t("Select the models to add to this provider.")}</DialogDescription>
        </DialogHeader>
        {!result && <p role="status" className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin motion-reduce:animate-none" />{t("Fetching model list…")}
        </p>}
        {result && "error" in result && <div className="space-y-3">
          <p role="alert" className="text-sm text-destructive">{t(result.error)}</p>
          <Button variant="outline" type="button" onClick={onRetry}>{t("Retry")}</Button>
        </div>}
        {result && "models" in result && (models.length === 0
          ? <p role="status" className="py-6 text-sm text-muted-foreground">{t("The provider returned no available models.")}</p>
          : <div className="min-w-0 space-y-2">
            <Input type="search" aria-label={t("Search models")} placeholder={t("Search models")} value={query} onChange={event => setQuery(event.target.value)} />
            {filtered.length === 0 ? <p role="status" className="py-6 text-sm text-muted-foreground">{t("No models match your search.")}</p>
              : <ul className="max-h-[min(40dvh,22rem)] divide-y overflow-y-auto rounded-md border">
                {filtered.map(model => {
                  const added = existingIds.has(model.id);
                  return <li key={model.id}>
                    <label className="flex min-w-0 items-center gap-3 px-3 py-2.5 transition-colors has-enabled:cursor-pointer has-enabled:hover:bg-accent">
                      <input type="checkbox" className="size-4 shrink-0 accent-primary" checked={added || selected.has(model.id)} disabled={added} onChange={() => toggle(model.id)} />
                      <span className="min-w-0 flex-1">
                        <span className="block break-all text-sm font-medium">{model.displayName}</span>
                        {model.displayName !== model.id && <span className="block break-all text-xs text-muted-foreground">{model.id}</span>}
                      </span>
                      {added && <span className="shrink-0 text-xs text-muted-foreground">{t("Already added")}</span>}
                    </label>
                  </li>;
                })}
              </ul>}
          </div>)}
        <DialogFooter className="gap-2 sm:space-x-0">
          <Button type="button" variant="outline" onClick={onClose}>{t("Cancel")}</Button>
          <Button type="button" disabled={!selected.size} onClick={() => onAdd(models.filter(model => selected.has(model.id) && !existingIds.has(model.id)))}>
            {selected.size ? t("Add selected ({count})", { count: selected.size }) : t("Add selected")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
