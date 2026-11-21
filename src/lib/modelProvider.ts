import { invoke } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { create } from "zustand";
import { isTauriRuntime } from "./apiAdapter";

export type ProviderProtocol = "anthropic" | "openAiChat" | "openAiResponses" | "gemini";

export interface ModelProvider {
  enabled: boolean;
  baseUrl: string;
  model: string;
  auth: "bearer" | "apiKey";
  protocol: ProviderProtocol;
  hasCredential: boolean;
}

type ProviderUpdate = Omit<ModelProvider, "hasCredential"> & { credential?: string };

const useProvider = create<{ model: string | null }>(() => ({ model: null }));

function publishProvider(provider: ModelProvider) {
  useProvider.setState({ model: provider.enabled ? provider.model : null });
  return provider;
}

// Use desktop IPC directly: apiCall logs arguments and falls back to HTTP.
export async function loadModelProvider() {
  return publishProvider(await invoke<ModelProvider>("get_model_provider"));
}

export async function saveModelProvider(update: ProviderUpdate) {
  return publishProvider(await invoke<ModelProvider>("save_model_provider", { update }));
}

export function useCustomModel() {
  const model = useProvider(state => state.model);
  useEffect(() => {
    if (isTauriRuntime()) void loadModelProvider().catch(() => undefined);
  }, []);
  return model;
}
