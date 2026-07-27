import { useRef, useEffect, useState } from "react";
import type { CSSProperties } from "react";
import type { ThreadTokenUsage } from "../../types";
import Bot from "lucide-react/dist/esm/icons/bot";
import ArrowUp from "lucide-react/dist/esm/icons/arrow-up";
import Check from "lucide-react/dist/esm/icons/check";
import ChevronDown from "lucide-react/dist/esm/icons/chevron-down";
import ChevronRight from "lucide-react/dist/esm/icons/chevron-right";
import CircleStop from "lucide-react/dist/esm/icons/circle-stop";
import Gauge from "lucide-react/dist/esm/icons/gauge";
import ImagePlus from "lucide-react/dist/esm/icons/image-plus";
import Plus from "lucide-react/dist/esm/icons/plus";
import RefreshCw from "lucide-react/dist/esm/icons/refresh-cw";
import Save from "lucide-react/dist/esm/icons/save";
import ShieldCheck from "lucide-react/dist/esm/icons/shield-check";
import Trash2 from "lucide-react/dist/esm/icons/trash-2";
import X from "lucide-react/dist/esm/icons/x";

type Props = {
  draft: string;
  onDraftChange: (text: string) => void;
  onSend: () => void;
  onStop: () => void;
  running: boolean;
  stopping: boolean;
  busy: boolean;
  disabled: boolean;
  tokenUsage: ThreadTokenUsage | null;
  providers?: ModelProviderSummary[];
  currentProviderId?: string | null;
  models?: ModelSummary[];
  catalogLoading?: boolean;
  catalogError?: string | null;
  onRefreshCatalog?: () => void;
  onWriteProvider?: (input: Record<string, unknown>) => Promise<void>;
  onSelectProvider?: (providerId: string) => void;
  selectedModelId?: string | null;
  onSelectModel?: (modelId: string) => void;
};

export type ModelProviderSummary = {
  id: string;
  name: string;
  kind: "builtIn" | "local" | "custom";
  isCurrent: boolean;
  modelCount: number;
  baseUrl?: string | null;
  envKey?: string | null;
  wireApi?: string;
  canEdit?: boolean;
  canDelete?: boolean;
  canFetchModels?: boolean;
  models?: { modelId: string; modelName?: string | null; contextWindow?: number | null }[];
};

export type ModelSummary = {
  id: string;
  displayName: string;
  model: string;
};

type ProviderCredentialMode = "environment" | "direct" | "preserve" | "none";
type ProviderGroupKey = "built-in" | "local" | "custom";

type ProviderDraft = {
  id: string;
  name: string;
  baseUrl: string;
  credentialMode: ProviderCredentialMode;
  envKey: string;
  apiKey: string;
  wireApi: string;
};

const EMPTY_PROVIDER_DRAFT: ProviderDraft = {
  id: "",
  name: "",
  baseUrl: "",
  credentialMode: "environment",
  envKey: "",
  apiKey: "",
  wireApi: "responses",
};

const DEFAULT_MODEL_CONTEXT_WINDOW = 128_000;

function formatTokens(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

function providerDescription(provider: ModelProviderSummary): string {
  if (provider.kind === "local") {
    const runtime = provider.id === "lmstudio"
      ? "LM Studio"
      : provider.id === "ollama"
        ? "Ollama"
        : provider.id;
    return provider.modelCount > 0
      ? `${runtime} · ${provider.modelCount} models`
      : `${runtime} local runtime`;
  }
  if (provider.modelCount > 0) return `${provider.modelCount} models`;
  return provider.kind === "builtIn" ? "Built-in catalog" : "No models fetched";
}

export default function Composer({ draft, onDraftChange, onSend, onStop, running, stopping, busy, disabled, tokenUsage, providers = [], currentProviderId = null, models = [], catalogLoading = false, catalogError = null, onRefreshCatalog, onWriteProvider, onSelectProvider, selectedModelId = null, onSelectModel }: Props) {
  const textRef = useRef<HTMLTextAreaElement>(null);
  const catalogRef = useRef<HTMLDivElement>(null);
  const composingRef = useRef(false);
  const [catalogOpen, setCatalogOpen] = useState(false);
  const [editingProvider, setEditingProvider] = useState<ModelProviderSummary | "new" | null>(null);
  const [providerDraft, setProviderDraft] = useState<ProviderDraft>(EMPTY_PROVIDER_DRAFT);
  const [contextDrafts, setContextDrafts] = useState<Record<string, string>>({});
  const [contextSaveState, setContextSaveState] = useState<"idle" | "saving" | "saved">("idle");
  const [openProviderGroup, setOpenProviderGroup] = useState<ProviderGroupKey | null>("custom");
  const [providerPendingDelete, setProviderPendingDelete] = useState<ModelProviderSummary | null>(null);
  const [deleteProviderState, setDeleteProviderState] = useState<"idle" | "deleting">("idle");
  const [deleteProviderError, setDeleteProviderError] = useState<string | null>(null);
  const currentProvider = providers.find((provider) => provider.id === currentProviderId);
  const builtInProviders = providers.filter((provider) => provider.kind === "builtIn");
  const localProviders = providers.filter((provider) => provider.kind === "local");
  const customProviders = providers.filter((provider) => provider.kind === "custom");
  const writeProvider = (input: Record<string, unknown>) => {
    void onWriteProvider?.(input).catch(() => undefined);
  };
  const providerDraftValid = Boolean(
    providerDraft.id.trim() &&
    providerDraft.name.trim() &&
    providerDraft.baseUrl.trim() &&
    (providerDraft.credentialMode !== "environment" || providerDraft.envKey.trim()) &&
    (providerDraft.credentialMode !== "direct" || providerDraft.apiKey.trim()),
  );
  const saveProviderDraft = () => {
    if (!providerDraftValid) return;
    void onWriteProvider?.({
      action: "upsert",
      ...providerDraft,
      select: editingProvider === "new",
    }).then(() => setEditingProvider(null)).catch(() => undefined);
  };

  const openProviderForm = (provider?: ModelProviderSummary) => {
    if (!provider) setCatalogOpen(false);
    setEditingProvider(provider ?? "new");
    setProviderDraft(provider ? {
      id: provider.id,
      name: provider.name,
      baseUrl: provider.baseUrl ?? "",
      credentialMode: provider.envKey ? "environment" : "preserve",
      envKey: provider.envKey ?? "",
      apiKey: "",
      wireApi: provider.wireApi ?? "responses",
    } : EMPTY_PROVIDER_DRAFT);
  };

  const openDeleteProviderDialog = (provider: ModelProviderSummary) => {
    setCatalogOpen(false);
    setDeleteProviderState("idle");
    setDeleteProviderError(null);
    setProviderPendingDelete(provider);
  };

  const closeDeleteProviderDialog = () => {
    if (deleteProviderState === "deleting") return;
    setProviderPendingDelete(null);
    setDeleteProviderError(null);
  };

  const deletePendingProvider = async () => {
    if (!providerPendingDelete || !onWriteProvider || deleteProviderState === "deleting") return;
    setDeleteProviderState("deleting");
    setDeleteProviderError(null);
    try {
      await onWriteProvider({ action: "delete", id: providerPendingDelete.id });
      setProviderPendingDelete(null);
    } catch (error) {
      setDeleteProviderState("idle");
      setDeleteProviderError(
        error instanceof Error && error.message.trim()
          ? error.message
          : "The provider could not be deleted. Please try again.",
      );
    }
  };

  useEffect(() => {
    if (!catalogOpen) return;
    const closeOnOutsideClick = (event: MouseEvent) => {
      if (!catalogRef.current?.contains(event.target as Node)) setCatalogOpen(false);
    };
    window.addEventListener("mousedown", closeOnOutsideClick);
    return () => window.removeEventListener("mousedown", closeOnOutsideClick);
  }, [catalogOpen]);

  useEffect(() => {
    if (editingProvider !== "new") return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setEditingProvider(null);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [editingProvider]);

  useEffect(() => {
    if (!providerPendingDelete || deleteProviderState === "deleting") return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setProviderPendingDelete(null);
        setDeleteProviderError(null);
      }
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [deleteProviderState, providerPendingDelete]);

  useEffect(() => {
    if (textRef.current) {
      textRef.current.style.height = "auto";
      textRef.current.style.height = `${Math.min(textRef.current.scrollHeight, 160)}px`;
    }
  }, [draft]);

  useEffect(() => {
    setContextDrafts({});
    setContextSaveState("idle");
  }, [currentProviderId]);

  const editableContextModels = currentProvider?.canEdit ? models : [];
  const contextDraftRows = editableContextModels.flatMap((model) => {
    if (!Object.prototype.hasOwnProperty.call(contextDrafts, model.model)) return [];
    const raw = contextDrafts[model.model] ?? "";
    const persisted = currentProvider?.models?.find((entry) => entry.modelId === model.model)
      ?.contextWindow ?? null;
    const value = Number(raw);
    const valid = raw.length > 0 && Number.isSafeInteger(value) && value >= 1_024;
    return [{
      modelId: model.model,
      contextWindow: value,
      valid,
      changed: valid && value !== persisted,
    }];
  });
  const invalidContextDraft = contextDraftRows.some((entry) => !entry.valid);
  const changedContexts = contextDraftRows.filter((entry) => entry.changed);
  const saveContexts = async () => {
    if (!currentProvider || !onWriteProvider || invalidContextDraft || changedContexts.length === 0) {
      return;
    }
    setContextSaveState("saving");
    try {
      await onWriteProvider({
        action: "contexts",
        id: currentProvider.id,
        contexts: changedContexts.map(({ modelId, contextWindow: nextContextWindow }) => ({
          modelId,
          contextWindow: nextContextWindow,
        })),
      });
      setContextSaveState("saved");
    } catch {
      setContextSaveState("idle");
    }
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (running) {
      if (!stopping) onStop();
      return;
    }
    if (!disabled && !busy) onSend();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (composingRef.current || e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229) {
      return;
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (running) {
        if (draft.trim()) onSend();
        return;
      }
      handleSubmit(e);
    }
  };

  const selectedModel = models.find((model) => model.id === selectedModelId);
  const selectedModelKey = selectedModel?.model ?? selectedModelId;
  const selectedModelContextWindow = selectedModelKey
    ? currentProvider?.models?.find((entry) => (
      entry.modelId === selectedModelKey || entry.modelId === selectedModel?.id
    ))?.contextWindow ?? null
    : null;
  const configuredContextWindow = selectedModelContextWindow
    ?? (currentProvider?.canEdit && selectedModelKey ? DEFAULT_MODEL_CONTEXT_WINDOW : null);
  const contextWindow = configuredContextWindow ?? tokenUsage?.modelContextWindow ?? 0;
  const lastTokens = tokenUsage?.last.totalTokens ?? 0;
  const contextUsage = tokenUsage
    ? lastTokens > 0 ? tokenUsage.last : tokenUsage.total
    : null;
  const usedTokens = contextUsage?.totalTokens ?? 0;
  const usedPercent = contextWindow > 0
    ? Math.min(Math.max((usedTokens / contextWindow) * 100, 0), 100)
    : null;
  const roundedPercent = usedPercent === null ? null : Math.round(usedPercent);
  const contextColor = usedPercent === null
    ? "#687183"
    : usedPercent >= 90
      ? "#ef6464"
      : usedPercent >= 70
        ? "#e9a04a"
        : "#82e63e";
  const contextLabel = usedPercent === null
    ? "Context usage unavailable"
    : `Context used ${roundedPercent}%: ${formatTokens(usedTokens)} of ${formatTokens(contextWindow)} tokens`;
  const providerEditor = editingProvider ? (
    <div
      className="web-provider-form"
      role={editingProvider === "new" ? undefined : "group"}
      aria-label={editingProvider === "new" ? undefined : "Provider settings"}
    >
      {editingProvider === "new" ? null : <h3>{`Edit ${editingProvider.name}`}</h3>}
      <label>
        ID
        <input
          value={providerDraft.id}
          disabled={editingProvider !== "new" && providerDraft.id === editingProvider.id}
          onChange={(event) => setProviderDraft((draft) => ({ ...draft, id: event.target.value }))}
          placeholder="deepseek"
          autoFocus={editingProvider === "new"}
          required
        />
      </label>
      <label>
        Name
        <input
          value={providerDraft.name}
          onChange={(event) => setProviderDraft((draft) => ({ ...draft, name: event.target.value }))}
          placeholder="DeepSeek"
          required
        />
      </label>
      <label>
        Base URL
        <input
          value={providerDraft.baseUrl}
          onChange={(event) => setProviderDraft((draft) => ({ ...draft, baseUrl: event.target.value }))}
          placeholder="https://api.deepseek.com"
          type="url"
          required
        />
      </label>
      <label>
        Credential source
        <select
          value={providerDraft.credentialMode}
          onChange={(event) => setProviderDraft((draft) => ({
            ...draft,
            credentialMode: event.target.value as ProviderCredentialMode,
          }))}
        >
          {editingProvider !== "new" ? <option value="preserve">Keep current credential</option> : null}
          <option value="environment">Environment variable</option>
          <option value="direct">Direct API key</option>
          <option value="none">No API key</option>
        </select>
      </label>
      {providerDraft.credentialMode === "environment" ? (
        <label>
          API key environment variable
          <input
            value={providerDraft.envKey}
            onChange={(event) => setProviderDraft((draft) => ({
              ...draft,
              envKey: event.target.value.toUpperCase(),
            }))}
            placeholder="DEEPSEEK_API_KEY"
            required
          />
          <small>Codex reads the secret from this variable when making requests.</small>
        </label>
      ) : null}
      {providerDraft.credentialMode === "direct" ? (
        <label>
          API key
          <input
            value={providerDraft.apiKey}
            onChange={(event) => setProviderDraft((draft) => ({
              ...draft,
              apiKey: event.target.value,
            }))}
            placeholder="Paste API key"
            type="password"
            autoComplete="off"
            required
          />
          <small>Stored directly in the Profile config. Environment variables are safer when available.</small>
        </label>
      ) : null}
      <label>
        Wire API
        <select
          value={providerDraft.wireApi}
          onChange={(event) => setProviderDraft((draft) => ({
            ...draft,
            wireApi: event.target.value,
          }))}
        >
          <option value="responses">Responses</option>
          <option value="chat">Chat</option>
        </select>
      </label>
      <div className="web-provider-form-actions">
        <button type="button" onClick={() => setEditingProvider(null)}>Cancel</button>
        <button
          type="button"
          className="is-primary"
          onClick={saveProviderDraft}
          disabled={catalogLoading || !providerDraftValid}
        >
          Save provider
        </button>
      </div>
    </div>
  ) : null;

  return (
    <>
      <form className="web-composer" onSubmit={handleSubmit}>
      <div className="web-composer-main">
        <span className="web-composer-utility" title="Image attachments are not available in Web mode" aria-hidden="true">
          <ImagePlus size={17} />
        </span>
        <div className="web-composer-inner">
          <textarea
            ref={textRef}
            value={draft}
            onChange={(e) => onDraftChange(e.target.value)}
            onKeyDown={handleKeyDown}
            onCompositionStart={() => {
              composingRef.current = true;
            }}
            onCompositionEnd={() => {
              composingRef.current = false;
            }}
            placeholder={running ? "Request follow-up changes…" : "Ask Codex to do something..."}
            disabled={disabled || busy}
            rows={1}
          />
        </div>
        <button
          type={running ? "button" : "submit"}
          className={`web-send-btn${running ? " is-stop" : ""}`}
          disabled={running ? stopping : disabled || busy}
          aria-label={running ? stopping ? "Stopping" : "Stop" : "Send"}
          onClick={running ? onStop : undefined}
        >
          {running ? (
            <CircleStop size={18} aria-hidden="true" />
          ) : (
            <ArrowUp size={20} strokeWidth={2.4} aria-hidden="true" />
          )}
        </button>
      </div>
      <div className="web-composer-footer" aria-label="Thread settings summary">
        <div className="web-composer-catalog" ref={catalogRef}>
          <button
            className="web-composer-chip web-composer-chip-button"
            type="button"
            aria-expanded={catalogOpen}
            aria-haspopup="dialog"
            onClick={() => setCatalogOpen((open) => !open)}
          >
            <Bot size={13} />
            {providers.find((provider) => provider.id === currentProviderId)?.name ?? "Codex"}
            <ChevronDown size={12} />
          </button>
          {catalogOpen ? (
            <section className="web-model-catalog" role="dialog" aria-label="Providers and models">
              <header>
                <div className="web-model-catalog-title">
                  <span className="web-model-catalog-mark"><Bot size={16} aria-hidden="true" /></span>
                  <span><strong>Provider & model</strong><small>Profile-wide catalog · Thread-specific selection</small></span>
                </div>
                <span className="web-model-catalog-actions">
                  <button type="button" onClick={() => openProviderForm()} disabled={catalogLoading}><Plus size={12} aria-hidden="true" />Add</button>
                  <button type="button" onClick={onRefreshCatalog} disabled={catalogLoading}><RefreshCw size={12} aria-hidden="true" />Refresh</button>
                </span>
              </header>
              {catalogError ? <p className="web-model-catalog-error">{catalogError}</p> : null}
              <div className="web-model-catalog-section web-provider-groups">
                <h3>Providers</h3>
                {providers.length === 0 ? <p>{catalogLoading ? "Loading providers…" : "No providers available"}</p> : null}
                {[
                  {
                    key: "built-in" as const,
                    label: "Built-in providers",
                    entries: builtInProviders,
                  },
                  {
                    key: "local" as const,
                    label: "Local providers",
                    entries: localProviders,
                  },
                  {
                    key: "custom" as const,
                    label: "Custom providers",
                    entries: customProviders,
                  },
                ].map((group) => {
                  if (group.entries.length === 0) return null;
                  const groupOpen = openProviderGroup === group.key;
                  const selected = group.entries.find((provider) => provider.id === currentProviderId);
                  return (
                    <section
                      className={`web-provider-group${groupOpen ? " is-open" : ""}`}
                      key={group.key}
                    >
                      <button
                        className="web-provider-group-toggle"
                        type="button"
                        aria-expanded={groupOpen}
                        aria-controls={`web-provider-group-${group.key}`}
                        onClick={() => setOpenProviderGroup((open) => (
                          open === group.key ? null : group.key
                        ))}
                      >
                        {groupOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                        <span>
                          <strong>{group.label}</strong>
                          <small>{group.entries.length} {group.entries.length === 1 ? "provider" : "providers"}</small>
                        </span>
                        {!groupOpen && selected ? <span className="web-selected-badge"><Check size={11} aria-hidden="true" />{selected.name}</span> : null}
                      </button>
                      {groupOpen ? (
                        <div
                          className="web-provider-group-list"
                          id={`web-provider-group-${group.key}`}
                        >
                          {group.entries.map((provider) => (
                            <div
                              className={`web-model-catalog-row web-provider-row${provider.id === currentProviderId ? " is-current" : ""}`}
                              key={provider.id}
                            >
                              <button
                                className="web-provider-select"
                                type="button"
                                disabled={catalogLoading}
                                aria-pressed={provider.id === currentProviderId}
                                onClick={() => {
                                  if (provider.id !== currentProviderId) onSelectProvider?.(provider.id);
                                }}
                              >
                                <span>
                                  <strong>{provider.name}</strong>
                                  <small>{providerDescription(provider)}</small>
                                </span>
                                {provider.id === currentProviderId ? <span className="web-selected-badge"><Check size={11} aria-hidden="true" />Selected</span> : null}
                              </button>
                              <span className="web-model-catalog-actions">
                                {provider.canEdit ? <button type="button" onClick={() => openProviderForm(provider)}>Edit</button> : null}
                                {provider.canFetchModels ? <button type="button" onClick={() => writeProvider({ action: "fetch", id: provider.id })}>Fetch</button> : null}
                                {provider.canEdit ? <button type="button" onClick={() => { setCatalogOpen(false); setEditingProvider("new"); setProviderDraft({ id: `${provider.id}-copy`, name: `${provider.name} Copy`, baseUrl: provider.baseUrl ?? "", credentialMode: provider.envKey ? "environment" : "none", envKey: provider.envKey ?? "", apiKey: "", wireApi: provider.wireApi ?? "responses" }); }}>Copy</button> : null}
                                {provider.canDelete ? <button className="is-danger" type="button" onClick={() => openDeleteProviderDialog(provider)}>Delete</button> : null}
                              </span>
                            </div>
                          ))}
                        </div>
                      ) : null}
                    </section>
                  );
                })}
              </div>
              {editingProvider && editingProvider !== "new" ? providerEditor : null}
              <div className="web-model-catalog-section">
                <div className="web-model-section-heading">
                  <span>
                    <h3>Models</h3>
                    {currentProvider ? <small>{currentProvider.name}</small> : null}
                  </span>
                  {editableContextModels.length > 0 ? (
                    <button
                      className="web-context-save"
                      type="button"
                      onClick={() => { void saveContexts(); }}
                      disabled={
                        catalogLoading
                        || contextSaveState === "saving"
                        || invalidContextDraft
                        || changedContexts.length === 0
                      }
                    >
                      {contextSaveState === "saved" && changedContexts.length === 0
                        ? <Check size={12} aria-hidden="true" />
                        : <Save size={12} aria-hidden="true" />}
                      {contextSaveState === "saving"
                        ? "Saving…"
                        : contextSaveState === "saved" && changedContexts.length === 0
                          ? "Saved"
                          : "Save contexts"}
                    </button>
                  ) : null}
                </div>
                {models.length === 0 ? <p>{catalogLoading ? "Loading models…" : "No models returned by this provider"}</p> : models.map((model) => (
                  <div className={`web-model-catalog-row web-model-catalog-model${selectedModelId === model.id ? " is-current" : ""}`} key={model.id}>
                    <button type="button" aria-pressed={selectedModelId === model.id} onClick={() => onSelectModel?.(model.id)}>
                    <span><strong>{model.displayName || model.model}</strong><small>{model.model}</small></span>
                    {selectedModelId === model.id ? <span className="web-selected-badge"><Check size={11} aria-hidden="true" />Selected</span> : null}
                    </button>
                    {currentProvider?.canEdit ? (
                      <label className="web-context-editor">
                        <span>Context</span>
                        <span className="web-context-field">
                          <input
                            aria-label={`Context window for ${model.model}`}
                            aria-invalid={
                              Object.prototype.hasOwnProperty.call(contextDrafts, model.model)
                              && (
                                !contextDrafts[model.model]
                                || Number(contextDrafts[model.model]) < 1_024
                              )
                            }
                            inputMode="numeric"
                            value={
                              contextDrafts[model.model]
                              ?? String(
                                currentProvider.models?.find(
                                  (entry) => entry.modelId === model.model,
                                )?.contextWindow
                                ?? DEFAULT_MODEL_CONTEXT_WINDOW,
                              )
                            }
                            disabled={catalogLoading || contextSaveState === "saving"}
                            onChange={(event) => {
                              setContextSaveState("idle");
                              setContextDrafts((drafts) => ({
                                ...drafts,
                                [model.model]: event.target.value.replace(/\D/g, ""),
                              }));
                            }}
                          />
                          <small>tokens</small>
                        </span>
                      </label>
                    ) : null}
                  </div>
                ))}
                {editableContextModels.length > 0 ? (
                  <div className={`web-context-save-note${invalidContextDraft ? " is-error" : ""}`}>
                    {invalidContextDraft
                      ? "Context windows must contain at least 1,024 tokens."
                      : changedContexts.length > 0
                        ? `${changedContexts.length} unsaved context ${changedContexts.length === 1 ? "change" : "changes"}`
                        : contextSaveState === "saved"
                          ? "All context windows saved."
                          : "Edit any context windows, then save them together."}
                  </div>
                ) : null}
              </div>
              <footer>Credentials are stored in the Codex Profile configuration or read from its environment. Secret values are never returned by the provider catalog.</footer>
            </section>
          ) : null}
        </div>
        <span className="web-composer-chip"><Gauge size={13} />medium<ChevronDown size={12} /></span>
        <span className="web-composer-chip"><ShieldCheck size={13} />Workspace access<ChevronDown size={12} /></span>
        <span className="web-composer-activity" tabIndex={0} aria-label={contextLabel}>
          <span
            className="web-composer-activity-ring"
            style={{
              "--context-used": usedPercent ?? 0,
              "--context-color": contextColor,
            } as CSSProperties}
          />
          <span className="web-composer-context-tooltip" role="tooltip">
            <strong>{usedPercent === null ? "Context unavailable" : `Context used ${roundedPercent}%`}</strong>
            <span>
              {usedPercent === null
                ? "Waiting for token usage data"
                : `${formatTokens(usedTokens)} / ${formatTokens(contextWindow)} tokens`}
            </span>
            {contextUsage ? (
              <span>
                Input {formatTokens(contextUsage.inputTokens)} · Output {formatTokens(contextUsage.outputTokens)}
              </span>
            ) : null}
          </span>
        </span>
      </div>
      </form>
      {editingProvider === "new" ? (
        <div className="web-provider-modal">
          <button
            type="button"
            className="web-provider-modal-backdrop"
            aria-label="Close add Provider dialog"
            onClick={() => setEditingProvider(null)}
          />
          <section
            className="web-provider-modal-panel"
            role="dialog"
            aria-modal="true"
            aria-labelledby="web-provider-modal-title"
          >
            <header className="web-provider-modal-header">
              <span>
                <h2 id="web-provider-modal-title">Add provider</h2>
                <p>Create a reusable Provider configuration for this Profile.</p>
              </span>
              <button
                type="button"
                className="web-provider-modal-close"
                aria-label="Close add Provider dialog"
                onClick={() => setEditingProvider(null)}
              >
                <X size={17} aria-hidden="true" />
              </button>
            </header>
            {providerEditor}
          </section>
        </div>
      ) : null}
      {providerPendingDelete ? (
        <div className="web-provider-modal">
          <button
            type="button"
            className="web-provider-modal-backdrop"
            aria-label="Close delete provider dialog"
            disabled={deleteProviderState === "deleting"}
            onClick={closeDeleteProviderDialog}
          />
          <section
            className="web-provider-modal-panel web-provider-delete-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="web-provider-delete-title"
            aria-describedby="web-provider-delete-description"
            aria-busy={deleteProviderState === "deleting"}
          >
            <header className="web-provider-modal-header">
              <span>
                <h2 id="web-provider-delete-title">{`Delete ${providerPendingDelete.name}?`}</h2>
                <p>This action cannot be undone.</p>
              </span>
              <button
                type="button"
                className="web-provider-modal-close"
                aria-label="Close delete provider dialog"
                disabled={deleteProviderState === "deleting"}
                onClick={closeDeleteProviderDialog}
              >
                <X size={17} aria-hidden="true" />
              </button>
            </header>
            <div className="web-provider-delete-body">
              <div className="web-provider-delete-identity">
                <span><Trash2 size={18} aria-hidden="true" /></span>
                <span>
                  <strong>{providerPendingDelete.name}</strong>
                  <small>{providerPendingDelete.id}</small>
                </span>
              </div>
              <p id="web-provider-delete-description">
                The Provider configuration and its stored credential will be removed from this Profile.
              </p>
              {deleteProviderError ? (
                <p className="web-provider-delete-error" role="alert">{deleteProviderError}</p>
              ) : null}
              <div className="web-provider-delete-actions">
                <button
                  type="button"
                  disabled={deleteProviderState === "deleting"}
                  onClick={closeDeleteProviderDialog}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="is-danger"
                  disabled={deleteProviderState === "deleting"}
                  onClick={() => { void deletePendingProvider(); }}
                >
                  <Trash2 size={14} aria-hidden="true" />
                  {deleteProviderState === "deleting" ? "Deleting…" : "Delete provider"}
                </button>
              </div>
            </div>
          </section>
        </div>
      ) : null}
    </>
  );
}
