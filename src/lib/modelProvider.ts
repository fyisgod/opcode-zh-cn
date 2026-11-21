import { invoke } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { create } from "zustand";
import { isTauriRuntime } from "./apiAdapter";

export type ProviderProtocol = "anthropic" | "openAiChat" | "openAiResponses" | "gemini";

export interface ModelProvider {
  id: string;
  name: string;
  enabled: boolean;
  baseUrl: string;
  models: string[];
  auth: "bearer" | "apiKey";
  protocol: ProviderProtocol;
  hasCredential: boolean;
}

export type ModelSelection =
  | { model: "sonnet" | "opus"; providerId?: undefined }
  | { model: string; providerId: string };

type ProviderUpdate = Omit<ModelProvider, "hasCredential"> & { credential?: string };

const useProviderStore = create<{ providers: ModelProvider[]; loaded: boolean; error: boolean }>(() => ({
  providers: [], loaded: false, error: false,
}));

let pendingLoad: Promise<ModelProvider[]> | undefined;

const publishProviders = (providers: ModelProvider[]) => {
  useProviderStore.setState({ providers, loaded: true, error: false });
  return providers;
};

// Use desktop IPC directly: apiCall logs arguments and falls back to HTTP.
export function loadModelProviders() {
  if (!pendingLoad) {
    pendingLoad = invoke<ModelProvider[]>("get_model_providers").then(publishProviders).catch(error => {
      useProviderStore.setState({ error: true });
      throw error;
    }).finally(() => { pendingLoad = undefined; });
  }
  return pendingLoad;
}

export async function saveModelProvider(update: ProviderUpdate) {
  const { id, providers } = await invoke<{ id: string; providers: ModelProvider[] }>("save_model_provider", { update });
  publishProviders(providers);
  return providers.find(provider => provider.id === id)!;
}

export async function removeModelProvider(id: string) {
  publishProviders(await invoke<ModelProvider[]>("remove_model_provider", { id }));
}

export function useModelProviders() {
  const state = useProviderStore();
  const desktop = isTauriRuntime();
  useEffect(() => {
    if (desktop && !useProviderStore.getState().loaded) void loadModelProviders().catch(() => undefined);
  }, [desktop]);
  return { ...state, loaded: !desktop || state.loaded };
}

export function initialModel(providers: ModelProvider[]): ModelSelection {
  const legacy = providers.find(provider => provider.id === "legacy-provider" && provider.enabled && provider.hasCredential);
  return legacy?.models[0] ? { model: legacy.models[0], providerId: legacy.id } : { model: "sonnet" };
}

export function savedProviderFor(selection: ModelSelection, providers: ModelProvider[]) {
  return providers.find(provider => provider.id === selection.providerId && provider.enabled
    && provider.hasCredential && provider.models.includes(selection.model));
}
