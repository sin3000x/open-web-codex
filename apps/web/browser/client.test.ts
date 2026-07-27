import { afterEach, describe, expect, it, vi } from "vitest";

import { PlatformClient } from "./client";

describe("PlatformClient", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("uses typed Run and message routes without a generic RPC surface", async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ run: { id: "run-1" } }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({
        status: "sent",
        thread_id: "thread-1",
        turn_id: "turn-1",
        thread_name: "hello",
      }), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("crypto", { randomUUID: () => "018f-idempotency-key" });
    const client = new PlatformClient({ baseUrl: "https://platform.test", token: "session-token" });

    await client.startRun("task/one", "feature/safe");
    await expect(client.sendMessage("task/one", "hello", {
      model: "deepseek-v4-flash",
      modelProvider: "deepseek",
    })).resolves.toMatchObject({
      thread_id: "thread-1",
      turn_id: "turn-1",
      thread_name: "hello",
    });

    expect(fetchMock.mock.calls[0]?.[0]).toBe("https://platform.test/api/tasks/task%2Fone/runs");
    expect(JSON.parse(String(fetchMock.mock.calls[0]?.[1]?.body))).toEqual({
      idempotency_key: "018f-idempotency-key",
      git_ref: "feature/safe",
      workspace_kind: "main",
      workspace_name: null,
      workspace_parent_run_id: null,
      workspace_group_run_id: null,
      copy_agents_md: false,
      fork_thread_id: null,
      fork_source_run_id: null,
    });
    expect(fetchMock.mock.calls[1]?.[0]).toBe("https://platform.test/api/tasks/task%2Fone/messages");
    expect(JSON.parse(String(fetchMock.mock.calls[1]?.[1]?.body))).toMatchObject({
      text: "hello",
      model: "deepseek-v4-flash",
      model_provider: "deepseek",
    });
    expect(fetchMock.mock.calls.every((call) => !String(call[0]).includes("/api/rpc"))).toBe(true);
  });

  it("falls back when crypto.randomUUID is unavailable", async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ run: { id: "run-1" } }), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("crypto", {});
    const client = new PlatformClient({ baseUrl: "https://platform.test", token: "session-token" });

    await client.startRun("task-one");

    const body = JSON.parse(String(fetchMock.mock.calls[0]?.[1]?.body));
    expect(body.idempotency_key).toMatch(/^idempotency-/);
  });

  it("authenticates WebSocket in the first frame instead of putting the token in its URL", () => {
    const instances: FakeSocket[] = [];
    class FakeSocket {
      onopen: (() => void) | null = null;
      onmessage: ((message: { data: string }) => void) | null = null;
      onclose: (() => void) | null = null;
      onerror: (() => void) | null = null;
      sent: string[] = [];
      constructor(readonly url: string | URL) {
        instances.push(this);
      }
      send(value: string) { this.sent.push(value); }
      close() {}
    }
    vi.stubGlobal("WebSocket", FakeSocket);
    const client = new PlatformClient({ baseUrl: "https://platform.test", token: "secret-session" });
    const unsubscribe = client.subscribe(() => undefined, () => undefined);
    const socket = instances[0];
    expect(String(socket?.url)).toBe("wss://platform.test/api/events/ws");
    expect(String(socket?.url)).not.toContain("secret-session");
    socket?.onopen?.();
    expect(JSON.parse(socket?.sent[0] ?? "{}")).toEqual({
      type: "authenticate",
      token: "secret-session",
    });
    unsubscribe();
  });

  it("paginates the complete durable Task event history by sequence", async () => {
    const firstPage = Array.from({ length: 200 }, (_, index) => ({ sequence: index + 1 }));
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify(firstPage), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify([{ sequence: 201 }]), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);
    const client = new PlatformClient({ baseUrl: "https://platform.test", token: "session-token" });

    await expect(client.listAllEvents("task/one")).resolves.toHaveLength(201);
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "https://platform.test/api/tasks/task%2Fone/events?limit=200&after_sequence=0",
    );
    expect(fetchMock.mock.calls[1]?.[0]).toBe(
      "https://platform.test/api/tasks/task%2Fone/events?limit=200&after_sequence=200",
    );
  });

  it("rejects cached HTML instead of treating it as an API payload", async () => {
    const fetchMock = vi.fn().mockResolvedValueOnce(new Response("<!doctype html>", {
      status: 200,
      headers: { "content-type": "text/html" },
    }));
    vi.stubGlobal("fetch", fetchMock);
    const client = new PlatformClient({ baseUrl: "https://platform.test" });

    await expect(client.health()).rejects.toThrow(
      "Server returned a non-JSON response for /api/health (HTTP 200",
    );
    expect(fetchMock.mock.calls[0]?.[1]?.cache).toBe("no-store");
  });

  it("uses username rather than email as the login identifier", async () => {
    const fetchMock = vi.fn().mockResolvedValueOnce(
      new Response(JSON.stringify({ session_token: "session-token" }), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const client = new PlatformClient({ baseUrl: "https://platform.test" });

    await client.login("test", "test");

    expect(fetchMock.mock.calls[0]?.[0]).toBe("https://platform.test/api/sessions");
    expect(JSON.parse(String(fetchMock.mock.calls[0]?.[1]?.body))).toEqual({
      username: "test",
      password: "test",
    });
  });

  it("creates an implicit local session without login credentials", async () => {
    const fetchMock = vi.fn().mockResolvedValueOnce(
      new Response(JSON.stringify({ session_token: "local-session" }), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const client = new PlatformClient({ baseUrl: "https://platform.test" });

    await expect(client.createLocalSession()).resolves.toMatchObject({
      session_token: "local-session",
    });
    expect(fetchMock).toHaveBeenCalledWith(
      "https://platform.test/api/sessions/local",
      expect.objectContaining({ method: "POST", cache: "no-store" }),
    );
    expect(fetchMock.mock.calls[0]?.[1]?.body).toBeUndefined();
  });

  it("reads, replaces, and reuses the unified maps configuration", async () => {
    const configuration = {
      configured: true,
      provider: "mapbox",
      mapboxAccessToken: "pk.public-token",
      canConfigure: true,
      updatedAt: "2026-07-23T00:00:00Z",
    };
    const fetchMock = vi.fn()
      .mockImplementation(() => Promise.resolve(
        new Response(JSON.stringify(configuration), { status: 200 }),
      ));
    vi.stubGlobal("fetch", fetchMock);
    const client = new PlatformClient({
      baseUrl: "https://platform.test",
      token: "session-token",
    });

    await expect(client.getMapsConfiguration()).resolves.toEqual(configuration);
    await expect(
      client.updateMapsConfiguration("mapbox", "pk.public-token"),
    ).resolves.toEqual(configuration);
    await expect(
      client.useMapsConfiguration(
        "http://127.0.0.1:43123/one-time-token",
      ),
    ).resolves.toEqual(configuration);

    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "https://platform.test/api/configuration/maps",
    );
    expect(fetchMock.mock.calls[1]?.[0]).toBe(
      "https://platform.test/api/configuration/maps",
    );
    expect(fetchMock.mock.calls[1]?.[1]?.method).toBe("PUT");
    expect(JSON.parse(String(fetchMock.mock.calls[1]?.[1]?.body))).toEqual({
      provider: "mapbox",
      apiKey: "pk.public-token",
    });
    expect(fetchMock.mock.calls[2]?.[0]).toBe(
      "https://platform.test/api/configuration/maps/use",
    );
    expect(JSON.parse(String(fetchMock.mock.calls[2]?.[1]?.body))).toEqual({
      elicitationUrl: "http://127.0.0.1:43123/one-time-token",
    });
  });

  it("never returns an active Google Maps key from the unified resource", async () => {
    const configuration = {
      configured: true,
      provider: "google",
      mapboxAccessToken: null,
      canConfigure: true,
      updatedAt: "2026-07-23T00:00:00Z",
    };
    const fetchMock = vi.fn()
      .mockImplementation(() => Promise.resolve(
        new Response(JSON.stringify(configuration), { status: 200 }),
      ));
    vi.stubGlobal("fetch", fetchMock);
    const client = new PlatformClient({
      baseUrl: "https://platform.test",
      token: "session-token",
    });

    await expect(client.getMapsConfiguration()).resolves.toEqual(configuration);
    await client.updateMapsConfiguration(
      "google",
      "google-secret",
      "http://127.0.0.1:43123/one-time-token",
    );
    expect(JSON.parse(String(fetchMock.mock.calls[1]?.[1]?.body))).toEqual({
      provider: "google",
      apiKey: "google-secret",
      elicitationUrl: "http://127.0.0.1:43123/one-time-token",
    });
    expect(configuration).not.toHaveProperty("apiKey");
  });
});
