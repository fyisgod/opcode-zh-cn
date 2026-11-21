import { useEffect, useState, type FormEvent } from "react";
import { Loader2, Plus, Save, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { t } from "@/lib/i18n";
import { loadModelProviders, removeModelProvider, saveModelProvider, useModelProviders, type ModelProvider, type ProviderProtocol } from "@/lib/modelProvider";
import { cn } from "@/lib/utils";

const protocols: Record<ProviderProtocol, { label: string; address: string }> = {
  anthropic: { label: "Anthropic Messages", address: "https://api.anthropic.com" },
  openAiChat: { label: "OpenAI Chat Completions", address: "https://api.openai.com/v1" },
  openAiResponses: { label: "OpenAI Responses", address: "https://api.openai.com/v1" },
  gemini: { label: "Gemini (OpenAI-compatible)", address: "https://generativelanguage.googleapis.com/v1beta/openai" },
};

const blankProvider: ModelProvider = {
  id: "", name: "", enabled: true, baseUrl: "", models: [], auth: "bearer", protocol: "anthropic", hasCredential: false,
};

export function ModelProviderSettings() {
  const { providers, loaded, error: loadError } = useModelProviders();
  const [provider, setProvider] = useState<ModelProvider>(blankProvider);
  const [lastSaved, setLastSaved] = useState<ModelProvider | null>(null);
  const [modelsText, setModelsText] = useState("");
  const [credential, setCredential] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);

  const select = (value?: ModelProvider) => {
    setProvider(value ?? blankProvider);
    setLastSaved(value ?? null);
    setModelsText(value?.models.join("\n") ?? "");
    setCredential("");
    setError(null);
    setSaved(false);
  };

  useEffect(() => {
    if (loaded) select(providers[0]);
  }, [loaded]);

  const edit = (changes: Partial<ModelProvider>) => {
    if ("protocol" in changes || "baseUrl" in changes || "auth" in changes) setCredential("");
    setProvider(previous => ({ ...previous, ...changes }));
    setError(null);
    setSaved(false);
  };

  const save = async () => {
    if (saving) return;
    setSaving(true);
    setError(null);
    setSaved(false);
    try {
      const stored = await saveModelProvider({
        id: provider.id, name: provider.name, enabled: provider.enabled, baseUrl: provider.baseUrl,
        models: modelsText.split(/\r?\n/), auth: provider.auth, protocol: provider.protocol,
        credential: credential.trim() || undefined,
      });
      select(stored);
      setSaved(true);
    } catch (err) {
      setError(typeof err === "string" ? err : "Could not save model provider settings.");
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (saving || !provider.id) return;
    setSaving(true);
    setConfirmRemove(false);
    try {
      await removeModelProvider(provider.id);
      select(providers.find(candidate => candidate.id !== provider.id));
    } catch (err) {
      setError(typeof err === "string" ? err : "Could not save model provider settings.");
    } finally {
      setSaving(false);
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    void save();
  };

  if (!loaded) {
    return loadError ? <div className="space-y-3">
      <p role="alert" className="text-sm text-destructive">{t("Could not load model provider settings.")}</p>
      <Button variant="outline" onClick={() => void loadModelProviders().catch(() => undefined)}>{t("Retry")}</Button>
    </div> : (
      <Loader2 aria-label={t("Loading...")} className="m-4 h-5 w-5 animate-spin motion-reduce:animate-none" />
    );
  }

  const canReuseCredential = lastSaved?.hasCredential && provider.protocol === lastSaved.protocol
    && provider.baseUrl.trim().replace(/\/+$/, "") === lastSaved.baseUrl && provider.auth === lastSaved.auth;

  return (
    <div className="grid gap-5 sm:grid-cols-[minmax(0,12rem)_minmax(0,1fr)] sm:gap-6">
      <div className="min-w-0 space-y-2">
        <Button type="button" variant="outline" className="w-full justify-start" disabled={saving} onClick={() => select()}>
          <Plus className="mr-2 size-4" />{t("Add provider")}
        </Button>
        <ul className="max-h-48 space-y-1 overflow-y-auto sm:max-h-[55vh]">
          {providers.map(item => (
            <li key={item.id}>
              <button type="button" disabled={saving} aria-pressed={provider.id === item.id}
                onClick={() => { if (provider.id !== item.id) select(item); }}
                className={cn("flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition-colors hover:bg-accent", provider.id === item.id && "bg-accent")}>
                <span className={cn("size-2 shrink-0 rounded-full", item.enabled ? "bg-primary" : "bg-muted-foreground")} aria-label={t(item.enabled ? "Enabled" : "Disabled")} />
                <span className="truncate" title={item.name}>{item.name}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    <form onSubmit={submit} className="min-w-0 max-w-2xl" autoComplete="off">
      <div className="flex items-start justify-between gap-4 border-b border-border pb-4">
        <h2 className="min-w-0 break-words text-base font-semibold">{lastSaved?.name || t("New provider")}</h2>
        <Switch id="model-provider-enabled" checked={provider.enabled} disabled={saving}
          aria-label={t("Enable provider")}
          onCheckedChange={enabled => edit({ enabled })} />
      </div>
      <fieldset disabled={saving} className="space-y-4 py-4">
        <div className="space-y-2">
          <Label htmlFor="model-provider-name">{t("Provider name")}</Label>
          <Input id="model-provider-name" value={provider.name} onChange={e => edit({ name: e.target.value })} />
        </div>
        <div className="space-y-2">
          <Label htmlFor="model-provider-protocol">{t("API protocol")}</Label>
          <Select value={provider.protocol} onValueChange={(protocol: ProviderProtocol) => edit({
            protocol,
            ...(protocol === "gemini" ? { baseUrl: protocols.gemini.address, auth: "bearer" } : {}),
          })} disabled={saving}>
            <SelectTrigger id="model-provider-protocol"><SelectValue /></SelectTrigger>
            <SelectContent>
              {Object.entries(protocols).map(([id, { label }]) => <SelectItem key={id} value={id}>{t(label)}</SelectItem>)}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-2">
          <Label htmlFor="model-provider-url">{t("Base URL")}</Label>
          <Input id="model-provider-url" placeholder={protocols[provider.protocol].address} spellCheck={false}
            value={provider.baseUrl} onChange={e => edit({ baseUrl: e.target.value })} />
        </div>
        <div className="space-y-2">
          <Label htmlFor="model-provider-models">{t("Model IDs (one per line)")}</Label>
          <Textarea id="model-provider-models" rows={3} placeholder="example-model" spellCheck={false}
            value={modelsText} onChange={e => { setModelsText(e.target.value); setError(null); setSaved(false); }} />
        </div>
        <div className="space-y-2">
          <Label htmlFor="model-provider-auth">{t("Authentication")}</Label>
          <Select value={provider.auth} onValueChange={(auth: ModelProvider["auth"]) => edit({ auth })} disabled={saving}>
            <SelectTrigger id="model-provider-auth"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="bearer">Authorization: Bearer</SelectItem>
              <SelectItem value="apiKey">x-api-key</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-2">
          <Label htmlFor="model-provider-credential">{t("API credential")}</Label>
          <Input id="model-provider-credential" type="password" autoComplete="new-password"
            placeholder={canReuseCredential ? t("Credential saved") : undefined}
            value={credential} onChange={e => { setCredential(e.target.value); setSaved(false); setError(null); }} />
        </div>
      </fieldset>
      <div className="flex flex-wrap items-center gap-2">
        <Button type="submit" disabled={saving}>
          {saving ? <Loader2 className="mr-2 h-4 w-4 animate-spin motion-reduce:animate-none" /> : <Save className="mr-2 h-4 w-4" />}
          {t("Save provider")}
        </Button>
        {provider.id && (
          <Button type="button" variant="ghost" disabled={saving} onClick={() => setConfirmRemove(true)}>
            <Trash2 className="mr-2 h-4 w-4" />{t("Remove provider")}
          </Button>
        )}
        {saved && <span role="status" className="text-sm text-muted-foreground">{t("Provider settings saved.")}</span>}
      </div>
      {error && <p role="alert" className="mt-3 text-sm text-destructive">{t(error)}</p>}
    </form>
      <Dialog open={confirmRemove} onOpenChange={setConfirmRemove}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("Remove provider?")}</DialogTitle>
            <DialogDescription>{t("Remove {name} and its saved credential?", { name: lastSaved?.name ?? provider.name })}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmRemove(false)}>{t("Cancel")}</Button>
            <Button variant="destructive" onClick={() => void remove()}>{t("Remove provider")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
