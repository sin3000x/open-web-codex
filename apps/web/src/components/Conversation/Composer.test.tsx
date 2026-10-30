// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import Composer from "./Composer";

afterEach(cleanup);

describe("Web Composer context usage", () => {
  it("disables Provider Fetch while the catalog request is loading", () => {
    const onWriteProvider = vi.fn();
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        catalogLoading
        currentProviderId="provider-1"
        providers={[{
          id: "provider-1",
          name: "Provider",
          kind: "custom",
          supportsFunctionTools: false,
          isCurrent: true,
          modelCount: 1,
          canFetchModels: true,
          models: [{ modelId: "model-1" }],
        }]}
        models={[{ id: "model-1", model: "model-1", displayName: "Model 1" }]}
        onWriteProvider={async (input) => { onWriteProvider(input); }}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Provider" }));
    const fetchButton = screen.getByRole("button", { name: "Fetch" });
    expect((fetchButton as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(fetchButton);
    expect(onWriteProvider).not.toHaveBeenCalled();
  });

  it("renders real context usage and hover details from token usage", () => {
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={{
          total: {
            totalTokens: 42_000,
            inputTokens: 38_000,
            cachedInputTokens: 0,
            outputTokens: 4_000,
            reasoningOutputTokens: 0,
          },
          last: {
            totalTokens: 32_000,
            inputTokens: 30_000,
            cachedInputTokens: 0,
            outputTokens: 2_000,
            reasoningOutputTokens: 0,
          },
          modelContextWindow: 128_000,
        }}
      />,
    );

    const indicator = screen.getByLabelText(
      "Context used 25%: 32,000 of 128,000 tokens",
    );
    expect(indicator).not.toBeNull();
    const ringStyle = indicator
      .querySelector(".web-composer-activity-ring")
      ?.getAttribute("style") ?? "";
    expect(ringStyle).toContain("--context-used: 25");
    expect(ringStyle).toContain("--context-color: #82e63e");
    expect(screen.getByText("32,000 / 128,000 tokens")).not.toBeNull();
    expect(screen.getByText("Input 30,000 · Output 2,000")).not.toBeNull();
  });

  it("keeps unreported usage unavailable even when the model context window is known", () => {
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        currentProviderId="deepseek"
        selectedModelId="deepseek-flash"
        providers={[{
          id: "deepseek",
          name: "DeepSeek",
          kind: "custom",
          supportsFunctionTools: false,
          isCurrent: true,
          modelCount: 1,
          canEdit: true,
          models: [{ modelId: "deepseek-flash", contextWindow: 64_000 }],
        }]}
        models={[{ id: "deepseek-flash", model: "deepseek-flash", displayName: "DeepSeek Flash" }]}
      />,
    );

    expect(screen.getByLabelText("Context usage unavailable")).not.toBeNull();
    expect(screen.queryByLabelText(/Context used 0%/)).toBeNull();
  });

  it("uses the selected model context setting for the context indicator", () => {
    const { rerender } = render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={{
          total: {
            totalTokens: 32_000,
            inputTokens: 30_000,
            cachedInputTokens: 0,
            outputTokens: 2_000,
            reasoningOutputTokens: 0,
          },
          last: {
            totalTokens: 32_000,
            inputTokens: 30_000,
            cachedInputTokens: 0,
            outputTokens: 2_000,
            reasoningOutputTokens: 0,
          },
          modelContextWindow: 128_000,
        }}
        currentProviderId="deepseek"
        selectedModelId="deepseek-flash"
        providers={[{
          id: "deepseek",
          name: "DeepSeek",
          kind: "custom",
          supportsFunctionTools: false,
          isCurrent: true,
          modelCount: 2,
          canEdit: true,
          models: [
            { modelId: "deepseek-flash", contextWindow: 64_000 },
            { modelId: "deepseek-pro", contextWindow: 256_000 },
          ],
        }]}
        models={[
          { id: "deepseek-flash", model: "deepseek-flash", displayName: "DeepSeek Flash" },
          { id: "deepseek-pro", model: "deepseek-pro", displayName: "DeepSeek Pro" },
        ]}
      />,
    );

    expect(screen.getByLabelText(
      "Context used 50%: 32,000 of 64,000 tokens",
    )).not.toBeNull();

    rerender(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={{
          total: {
            totalTokens: 32_000,
            inputTokens: 30_000,
            cachedInputTokens: 0,
            outputTokens: 2_000,
            reasoningOutputTokens: 0,
          },
          last: {
            totalTokens: 32_000,
            inputTokens: 30_000,
            cachedInputTokens: 0,
            outputTokens: 2_000,
            reasoningOutputTokens: 0,
          },
          modelContextWindow: 128_000,
        }}
        currentProviderId="deepseek"
        selectedModelId="deepseek-pro"
        providers={[{
          id: "deepseek",
          name: "DeepSeek",
          kind: "custom",
          supportsFunctionTools: false,
          isCurrent: true,
          modelCount: 2,
          canEdit: true,
          models: [
            { modelId: "deepseek-flash", contextWindow: 64_000 },
            { modelId: "deepseek-pro", contextWindow: 256_000 },
          ],
        }]}
        models={[
          { id: "deepseek-flash", model: "deepseek-flash", displayName: "DeepSeek Flash" },
          { id: "deepseek-pro", model: "deepseek-pro", displayName: "DeepSeek Pro" },
        ]}
      />,
    );

    expect(screen.getByLabelText(
      "Context used 13%: 32,000 of 256,000 tokens",
    )).not.toBeNull();
  });

  it("does not invent usage when the context window is unavailable", () => {
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
      />,
    );

    expect(screen.getByLabelText("Context usage unavailable")).not.toBeNull();
    expect(screen.getByText("Waiting for token usage data")).not.toBeNull();
  });

  it("does not send when Enter confirms an IME composition", () => {
    const onSend = vi.fn();
    const view = render(
      <Composer
        draft="English"
        onDraftChange={vi.fn()}
        onSend={onSend}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
      />,
    );

    const input = view.container.querySelector("textarea");
    expect(input).not.toBeNull();
    if (!input) return;
    fireEvent.compositionStart(input);
    fireEvent.keyDown(input, { key: "Enter", code: "Enter", keyCode: 13 });
    expect(onSend).not.toHaveBeenCalled();

    fireEvent.compositionEnd(input);
    fireEvent.keyDown(input, { key: "Enter", code: "Enter", keyCode: 13 });
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it("turns the send action into a working stop button", () => {
    const onSend = vi.fn();
    const onStop = vi.fn();
    const view = render(
      <Composer
        draft="queued follow-up"
        onDraftChange={vi.fn()}
        onSend={onSend}
        onStop={onStop}
        running
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(onStop).toHaveBeenCalledTimes(1);
    expect(onSend).not.toHaveBeenCalled();

    view.rerender(
      <Composer
        draft="queued follow-up"
        onDraftChange={vi.fn()}
        onSend={onSend}
        onStop={onStop}
        running
        stopping
        busy={false}
        disabled={false}
        tokenUsage={null}
      />,
    );
    expect(screen.getByRole("button", { name: "Stopping" }).hasAttribute("disabled")).toBe(true);
  });

  it("queues Enter input while the stop button remains independent", () => {
    const onSend = vi.fn();
    const onStop = vi.fn();
    render(
      <Composer
        draft="Follow up after this task"
        onDraftChange={vi.fn()}
        onSend={onSend}
        onStop={onStop}
        running
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
      />,
    );

    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter", code: "Enter" });
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onStop).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(onStop).toHaveBeenCalledTimes(1);
  });
});

describe("Web Composer provider credentials", () => {
  it("does not report an empty built-in catalog as zero models", () => {
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        providers={[{
          id: "openai",
          name: "OpenAI",
          kind: "builtIn",
          supportsFunctionTools: true,
          isCurrent: false,
          modelCount: 0,
          canEdit: false,
          canDelete: false,
          canFetchModels: false,
        }]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Codex" }));
    fireEvent.click(screen.getByRole("button", { name: /Built-in providers/i }));
    expect(screen.getByText("Built-in catalog")).not.toBeNull();
    expect(screen.queryByText("0 models")).toBeNull();
  });

  it("groups providers, collapses built-ins, and selects a provider by row click", () => {
    const onSelectProvider = vi.fn();
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        currentProviderId="openai"
        providers={[
          {
            id: "openai",
            name: "OpenAI",
            kind: "builtIn",
            supportsFunctionTools: true,
            isCurrent: true,
            modelCount: 2,
          },
          {
            id: "deepseek",
            name: "DeepSeek",
            kind: "custom",
            supportsFunctionTools: false,
            isCurrent: false,
            modelCount: 1,
          },
          {
            id: "lmstudio",
            name: "gpt-oss",
            kind: "local",
            supportsFunctionTools: false,
            isCurrent: false,
            modelCount: 0,
          },
        ]}
        onSelectProvider={onSelectProvider}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /OpenAI/i }));
    const builtIns = screen.getByRole("button", { name: /Built-in providers.*OpenAI/i });
    expect(builtIns.getAttribute("aria-expanded")).toBe("false");
    expect(screen.getByRole("button", { name: /Custom providers/i }).getAttribute("aria-expanded"))
      .toBe("true");
    const localProviders = screen.getByRole("button", { name: /Local providers/i });
    expect(localProviders.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("button", { name: "Use" })).toBeNull();

    fireEvent.click(localProviders);
    expect(localProviders.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByRole("button", { name: /Custom providers/i }).getAttribute("aria-expanded"))
      .toBe("false");
    expect(screen.getByRole("button", { name: /gpt-oss.*LM Studio local runtime/i })).not.toBeNull();
    expect(screen.queryByRole("button", { name: /DeepSeek.*1 models/i })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Custom providers/i }));
    expect(localProviders.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(screen.getByRole("button", { name: /DeepSeek.*1 models/i }));
    expect(onSelectProvider).toHaveBeenCalledWith("deepseek");
  });

  it("confirms provider deletion in an in-app dialog instead of a browser alert", async () => {
    const onWriteProvider = vi.fn(async () => undefined);
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        currentProviderId="deepseek"
        providers={[{
          id: "deepseek",
          name: "DeepSeek",
          kind: "custom",
          supportsFunctionTools: false,
          isCurrent: true,
          modelCount: 1,
          canDelete: true,
        }]}
        onWriteProvider={onWriteProvider}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "DeepSeek" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    const firstDialog = screen.getByRole("dialog", { name: "Delete DeepSeek?" });
    expect(confirmSpy).not.toHaveBeenCalled();
    expect(onWriteProvider).not.toHaveBeenCalled();

    fireEvent.click(within(firstDialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: "Delete DeepSeek?" })).toBeNull();
    expect(onWriteProvider).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "DeepSeek" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    fireEvent.click(
      within(screen.getByRole("dialog", { name: "Delete DeepSeek?" }))
        .getByRole("button", { name: "Delete provider" }),
    );

    await waitFor(() => {
      expect(onWriteProvider).toHaveBeenCalledWith({
        action: "delete",
        id: "deepseek",
      });
      expect(screen.queryByRole("dialog", { name: "Delete DeepSeek?" })).toBeNull();
    });
    expect(confirmSpy).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });

  it("renders a model name and id as separate text rows", () => {
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        models={[{
          id: "deepseek-v4-flash",
          model: "deepseek-v4-flash",
          displayName: "DeepSeek V4 Flash",
        }]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Codex" }));
    const name = screen.getByText("DeepSeek V4 Flash");
    const id = screen.getByText("deepseek-v4-flash");
    expect(name.tagName).toBe("STRONG");
    expect(id.tagName).toBe("SMALL");
    expect(name.parentElement).toBe(id.parentElement);
  });

  it("saves all edited context windows with one action", async () => {
    const onWriteProvider = vi.fn(async () => undefined);
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        currentProviderId="deepseek"
        selectedModelId="deepseek-flash"
        providers={[{
          id: "deepseek",
          name: "DeepSeek",
          kind: "custom",
          supportsFunctionTools: false,
          isCurrent: true,
          modelCount: 2,
          canEdit: true,
          models: [
            { modelId: "deepseek-flash", contextWindow: 64_000 },
            { modelId: "deepseek-pro", contextWindow: 128_000 },
          ],
        }]}
        models={[
          { id: "deepseek-flash", model: "deepseek-flash", displayName: "DeepSeek Flash" },
          { id: "deepseek-pro", model: "deepseek-pro", displayName: "DeepSeek Pro" },
        ]}
        onWriteProvider={onWriteProvider}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "DeepSeek" }));
    expect(screen.queryByRole("button", { name: "Set context" })).toBeNull();
    expect(screen.getByRole("button", { name: /DeepSeek Flash.*Selected/i }).getAttribute("aria-pressed"))
      .toBe("true");

    fireEvent.change(screen.getByLabelText("Context window for deepseek-flash"), {
      target: { value: "96000" },
    });
    fireEvent.change(screen.getByLabelText("Context window for deepseek-pro"), {
      target: { value: "192000" },
    });
    expect(screen.getByText("2 unsaved context changes")).not.toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Save contexts" }));

    await waitFor(() => {
      expect(onWriteProvider).toHaveBeenCalledTimes(1);
      expect(onWriteProvider).toHaveBeenCalledWith({
        action: "contexts",
        id: "deepseek",
        contexts: [
          { modelId: "deepseek-flash", contextWindow: 96_000 },
          { modelId: "deepseek-pro", contextWindow: 192_000 },
        ],
      });
    });
  });

  it("uses 128000 as the actual context value when no value is configured", () => {
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        currentProviderId="deepseek"
        providers={[{
          id: "deepseek",
          name: "DeepSeek",
          kind: "custom",
          supportsFunctionTools: false,
          isCurrent: true,
          modelCount: 1,
          canEdit: true,
          models: [{ modelId: "deepseek-flash", contextWindow: null }],
        }]}
        models={[{
          id: "deepseek-flash",
          model: "deepseek-flash",
          displayName: "DeepSeek Flash",
        }]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "DeepSeek" }));
    const input = screen.getByLabelText(
      "Context window for deepseek-flash",
    ) as HTMLInputElement;

    expect(input.value).toBe("128000");
    expect(input.getAttribute("placeholder")).toBeNull();
  });

  it("submits a direct API key without displaying it in the provider catalog", async () => {
    const onWriteProvider = vi.fn(async () => undefined);
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        onWriteProvider={onWriteProvider}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Codex" }));
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    const dialog = screen.getByRole("dialog", { name: "Add provider" });
    const editor = within(dialog);
    expect(screen.queryByRole("dialog", { name: "Providers and models" })).toBeNull();
    fireEvent.change(editor.getByLabelText("ID"), { target: { value: "deepseek" } });
    fireEvent.change(editor.getByLabelText("Name"), { target: { value: "DeepSeek" } });
    fireEvent.change(editor.getByLabelText("Base URL"), {
      target: { value: "https://api.deepseek.com" },
    });
    fireEvent.change(editor.getByLabelText("Credential source"), {
      target: { value: "direct" },
    });
    fireEvent.change(editor.getByPlaceholderText("Paste API key"), {
      target: { value: "test-direct-key" },
    });
    fireEvent.click(editor.getByRole("button", { name: "Save provider" }));

    await waitFor(() => {
      expect(onWriteProvider).toHaveBeenCalledWith({
        action: "upsert",
        id: "deepseek",
        name: "DeepSeek",
        baseUrl: "https://api.deepseek.com",
        credentialMode: "direct",
        envKey: "",
        apiKey: "test-direct-key",
        wireApi: "responses",
        select: true,
      });
    });
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Add provider" })).toBeNull();
    });
  });

  it("enables function tools implicitly for Chat without rendering a second capability switch", async () => {
    const onWriteProvider = vi.fn(async () => undefined);
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        onWriteProvider={onWriteProvider}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Codex" }));
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    const editor = within(screen.getByRole("dialog", { name: "Add provider" }));
    fireEvent.change(editor.getByLabelText("ID"), { target: { value: "deepseek" } });
    fireEvent.change(editor.getByLabelText("Name"), { target: { value: "DeepSeek" } });
    fireEvent.change(editor.getByLabelText("Base URL"), {
      target: { value: "https://api.deepseek.com" },
    });
    fireEvent.change(editor.getByLabelText("Credential source"), {
      target: { value: "none" },
    });
    fireEvent.change(editor.getByLabelText("Wire API"), { target: { value: "chat" } });
    expect(editor.queryByLabelText("Function tools")).toBeNull();
    expect(editor.getByText("Function tools are enabled automatically for Chat providers."))
      .toBeTruthy();

    fireEvent.change(editor.getByLabelText("Wire API"), { target: { value: "responses" } });
    expect(editor.queryByLabelText("Function tools")).toBeNull();
    expect(editor.queryByText("Function tools are enabled automatically for Chat providers."))
      .toBeNull();
    fireEvent.change(editor.getByLabelText("Wire API"), { target: { value: "chat" } });
    fireEvent.click(editor.getByRole("button", { name: "Save provider" }));

    await waitFor(() => expect(onWriteProvider).toHaveBeenCalledWith(expect.objectContaining({
      wireApi: "chat",
    })));
  });

  it("keeps the automatic Chat capability separate from the model Tool search switch", () => {
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        currentProviderId="deepseek"
        providers={[{
          id: "deepseek",
          name: "DeepSeek",
          kind: "custom",
          supportsFunctionTools: false,
          isCurrent: true,
          modelCount: 1,
          canEdit: true,
          models: [{ modelId: "deepseek-v4-flash", supportsSearchTool: true }],
        }]}
        models={[{
          id: "deepseek-v4-flash",
          model: "deepseek-v4-flash",
          displayName: "DeepSeek V4 Flash",
        }]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "DeepSeek" }));
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const editor = within(screen.getByRole("group", { name: "Provider settings" }));
    expect((editor.getByLabelText("Wire API") as HTMLSelectElement).value).toBe("responses");
    expect(editor.queryByLabelText("Function tools")).toBeNull();
    fireEvent.change(editor.getByLabelText("Wire API"), { target: { value: "chat" } });
    expect(editor.queryByLabelText("Function tools")).toBeNull();
    expect(editor.getByText("Function tools are enabled automatically for Chat providers."))
      .toBeTruthy();
    expect((screen.getByLabelText("Tool search for deepseek-v4-flash") as HTMLInputElement).checked)
      .toBe(true);
  });

  it("copies a Chat Provider without exposing a function-tool choice", () => {
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        providers={[{
          id: "deepseek",
          name: "DeepSeek",
          kind: "custom",
          supportsFunctionTools: true,
          isCurrent: true,
          modelCount: 0,
          wireApi: "chat",
          canEdit: true,
        }]}
        currentProviderId="deepseek"
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "DeepSeek" }));
    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    const editor = within(screen.getByRole("dialog", { name: "Add provider" }));
    expect((editor.getByLabelText("Wire API") as HTMLSelectElement).value).toBe("chat");
    expect(editor.queryByLabelText("Function tools")).toBeNull();
    expect(editor.getByText("Function tools are enabled automatically for Chat providers."))
      .toBeTruthy();
  });

  it("keeps the selected-map revision attachment without a generic processed-result picker", () => {
    render(
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onStop={vi.fn()}
        running={false}
        stopping={false}
        busy={false}
        disabled={false}
        tokenUsage={null}
        mapCardAttachment={{ ref: "map-card-1", title: "Coverage" }}
      />,
    );

    expect(screen.getByText("基于地图：Coverage")).toBeTruthy();
    expect(screen.queryByText("Reuse processed result")).toBeNull();
  });
});
