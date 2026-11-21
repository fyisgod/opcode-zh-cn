import { useEffect, useState, type FormEvent } from "react";
import { Loader2, Save, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { t } from "@/lib/i18n";
import { loadModelProvider, saveModelProvider, type ModelProvider, type ProviderProtocol } from "@/lib/modelProvider";

const protocols: Record<ProviderProtocol, { label: string; address: string }> = {
  anthropic: { label: "Anthropic Messages", address: "https://api.anthropic.com" },
  openAiChat: { label: "OpenAI Chat Completions", address: "https://api.openai.com/v1" },
  openAiResponses: { label: "OpenAI Responses", address: "https://api.openai.com/v1" },
  gemini: { label: "Gemini (OpenAI-compatible)", address: "https://generativelanguage.googleapis.com/v1beta/openai" },
};

export function ModelProviderSettings() {
  const [provider, setProvider] = useState<ModelProvider | null>(null);
  const [lastSaved, setLastSaved] = useState<ModelProvider | null>(null);
  const [credential, setCredential] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let active = true;
    loadModelProvider().then(value => {
      if (active) {
        setProvider(value);
        setLastSaved(value);
      }
    }).catch(() => {
      if (active) setError("Could not load model provider settings.");
    });
    return () => { active = false; };
  }, []);

  const edit = (changes: Partial<ModelProvider>) => {
    if ("protocol" in changes || "baseUrl" in changes || "auth" in changes) setCredential("");
    setProvider(previous => previous && { ...previous, ...changes });
    setError(null);
    setSaved(false);
  };

  const save = async (removeCredential = false) => {
    if (!provider || saving) return;
    setSaving(true);
    setError(null);
    setSaved(false);
    try {
      const stored = await saveModelProvider({
        enabled: !removeCredential && provider.enabled,
        baseUrl: provider.baseUrl,
        model: provider.model,
        auth: provider.auth,
        protocol: provider.protocol,
        credential: removeCredential ? "" : credential.trim() || undefined,
      });
      setProvider(stored);
      setLastSaved(stored);
      setCredential("");
      setSaved(true);
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

  if (!provider) {
    return error ? <p role="alert" className="text-sm text-destructive">{t(error)}</p> : (
      <Loader2 aria-label={t("Loading...")} className="m-4 h-5 w-5 animate-spin motion-reduce:animate-none" />
    );
  }

  const canReuseCredential = lastSaved?.hasCredential && provider.protocol === lastSaved.protocol
    && provider.baseUrl.trim().replace(/\/+$/, "") === lastSaved.baseUrl && provider.auth === lastSaved.auth;

  return (
    <form onSubmit={submit} className="max-w-2xl" autoComplete="off">
      <div className="flex items-start justify-between gap-4 border-b border-border pb-4">
        <Label htmlFor="model-provider-enabled" className="text-base font-semibold">{t("Custom provider")}</Label>
        <Switch id="model-provider-enabled" checked={provider.enabled} disabled={saving}
          onCheckedChange={enabled => edit({ enabled })} />
      </div>
      <fieldset disabled={saving} className="space-y-4 py-4">
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
          <Label htmlFor="model-provider-model">{t("Model ID")}</Label>
          <Input id="model-provider-model" placeholder="example-model" spellCheck={false}
            value={provider.model} onChange={e => edit({ model: e.target.value })} />
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
        {provider.hasCredential && (
          <Button type="button" variant="ghost" disabled={saving} onClick={() => void save(true)}>
            <Trash2 className="mr-2 h-4 w-4" />{t("Disable and remove credential")}
          </Button>
        )}
        {saved && <span role="status" className="text-sm text-muted-foreground">{t("Provider settings saved.")}</span>}
      </div>
      {error && <p role="alert" className="mt-3 text-sm text-destructive">{t(error)}</p>}
    </form>
  );
}
