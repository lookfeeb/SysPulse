import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type AiMcpServerItem } from "@/bindings";
import { MCP_SCAN_INTERVAL_MS, useMcpStore } from "./mcpStore";

vi.mock("@/bindings", () => ({ commands: { aiGetMcpServers: vi.fn(), aiGetMcpOverview: vi.fn(), aiMcpOauthStatus: vi.fn() } }));
const server: AiMcpServerItem = { client: "codex", name: "remote", type: "http", detail: "https://example.invalid/mcp", disabled: false };
const status = { oauthSupported: true, authorized: true, expiresAt: 123, expiringSoon: false, expired: false, refreshFailed: false, needsReauth: false, message: null };
const deferred = <T>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
};
beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  useMcpStore.setState(useMcpStore.getInitialState());
  vi.mocked(commands.aiGetMcpServers).mockImplementation(async ({ client }) => client === "codex" ? [server] : []);
  vi.mocked(commands.aiMcpOauthStatus).mockResolvedValue(status);
});
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); });

describe("MCP session cache", () => {
  it("loads once across repeated initialization and subscriptions, without duplicate overview reads", async () => {
    await Promise.all([useMcpStore.getState().initialize(), useMcpStore.getState().initialize()]);
    const unsubscribe = useMcpStore.subscribe(() => undefined);
    await useMcpStore.getState().initialize();
    unsubscribe();
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(3);
    expect(commands.aiGetMcpOverview).not.toHaveBeenCalled();
    expect(commands.aiMcpOauthStatus).toHaveBeenCalledTimes(1);
    expect(useMcpStore.getState().overview).toMatchObject({ totalServers: 1, enabledServers: 1 });
  });

  it("deduplicates manual refreshes and keeps selected client and filters", async () => {
    await useMcpStore.getState().initialize();
    useMcpStore.setState({ activeClient: "kiro", serverQuery: "query", serverFilter: "remote" });
    await useMcpStore.getState().initialize();
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(3);
    await Promise.all([useMcpStore.getState().refresh(), useMcpStore.getState().refresh()]);
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(6);
    expect(useMcpStore.getState()).toMatchObject({ activeClient: "kiro", serverQuery: "query", serverFilter: "remote" });
  });

  it("silently retries a failed startup on the next scheduled scan", async () => {
    vi.mocked(commands.aiGetMcpServers).mockRejectedValueOnce(new Error("offline"));
    expect(await useMcpStore.getState().initialize()).toBe(false);
    await useMcpStore.getState().initialize();
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(3);
    expect(useMcpStore.getState()).toMatchObject({ overview: null, loadError: null, loading: false });
    await vi.advanceTimersByTimeAsync(MCP_SCAN_INTERVAL_MS);
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(6);
    expect(useMcpStore.getState().overview?.totalServers).toBe(1);
    expect(useMcpStore.getState().loadError).toBeNull();
  });

  it("preserves the last complete snapshot on a failed refresh", async () => {
    await useMcpStore.getState().initialize();
    const cached = useMcpStore.getState().serversByClient;
    vi.mocked(commands.aiGetMcpServers).mockRejectedValueOnce(new Error("unavailable"));
    expect(await useMcpStore.getState().refresh()).toBe(false);
    expect(useMcpStore.getState().serversByClient).toBe(cached);
    expect(useMcpStore.getState().oauthByServer["codex:remote"]).toEqual(status);
    await useMcpStore.getState().initialize();
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(6);
  });

  it("completes an OAuth failure with an explicit unknown status", async () => {
    vi.mocked(commands.aiMcpOauthStatus).mockRejectedValue(new Error("timeout"));
    await useMcpStore.getState().initialize();
    expect(useMcpStore.getState().loading).toBe(false);
    expect(useMcpStore.getState().oauthByServer["codex:remote"]).toMatchObject({ oauthSupported: null, message: "timeout" });
  });

  it("keeps the action guard across subscribers and blocks conflicting refreshes", async () => {
    const job = deferred<AiMcpServerItem[]>();
    vi.mocked(commands.aiGetMcpServers).mockImplementationOnce(() => job.promise);
    const loading = useMcpStore.getState().refresh();
    expect(useMcpStore.getState().beginAction("toggle")).toBe(false);
    job.resolve([server]); await loading;
    expect(useMcpStore.getState().beginAction("toggle")).toBe(true);
    const unsubscribe = useMcpStore.subscribe(() => undefined); unsubscribe();
    expect(useMcpStore.getState().beginAction("delete")).toBe(false);
    expect(await useMcpStore.getState().refresh()).toBe(false);
    useMcpStore.getState().finishAction("wrong-action");
    expect(useMcpStore.getState().exclusiveAction).toBe("toggle");
    useMcpStore.getState().finishAction("toggle");
    expect(useMcpStore.getState().beginAction("refresh")).toBe(true);
    expect(await useMcpStore.getState().refresh()).toBe(true);
    useMcpStore.getState().finishAction("refresh");
  });

  it("updates toggle/delete results and counts locally without rereading configuration", async () => {
    await useMcpStore.getState().initialize();
    useMcpStore.getState().updateServers((current) => ({ ...current, codex: [{ ...server, disabled: true }] }));
    expect(useMcpStore.getState().overview?.enabledServers).toBe(0);
    useMcpStore.getState().updateServers((current) => ({ ...current, codex: [] }));
    expect(useMcpStore.getState().overview?.totalServers).toBe(0);
    expect(useMcpStore.getState().oauthByServer).toEqual({});
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(3);
  });

  it("reconciles an explicit import only for its target client", async () => {
    await useMcpStore.getState().initialize();
    const codex = useMcpStore.getState().serversByClient.codex;
    vi.mocked(commands.aiGetMcpServers).mockResolvedValueOnce([{ ...server, client: "kiro" }]);
    await useMcpStore.getState().syncClient("kiro");
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(4);
    expect(commands.aiGetMcpServers).toHaveBeenLastCalledWith({ client: "kiro" });
    expect(useMcpStore.getState().serversByClient.codex).toBe(codex);
    expect(useMcpStore.getState().overview?.totalServers).toBe(2);
  });

  it("scans every two hours from startup even after manual refreshes and route changes", async () => {
    await useMcpStore.getState().initialize();
    await vi.advanceTimersByTimeAsync(MCP_SCAN_INTERVAL_MS / 2);
    await useMcpStore.getState().refresh();
    const unsubscribe = useMcpStore.subscribe(() => undefined); unsubscribe();
    await useMcpStore.getState().initialize();
    await vi.advanceTimersByTimeAsync(MCP_SCAN_INTERVAL_MS / 2 - 1);
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(6);
    await vi.advanceTimersByTimeAsync(1);
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(9);
    await vi.advanceTimersByTimeAsync(MCP_SCAN_INTERVAL_MS);
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(12);
  });

  it("preserves the visible snapshot and controls during slow or failed background scans", async () => {
    await useMcpStore.getState().initialize();
    const cached = useMcpStore.getState().serversByClient;
    const job = deferred<AiMcpServerItem[]>();
    vi.mocked(commands.aiGetMcpServers).mockImplementationOnce(() => job.promise);
    await vi.advanceTimersByTimeAsync(MCP_SCAN_INTERVAL_MS);
    expect(useMcpStore.getState()).toMatchObject({ loading: false, exclusiveAction: null, loadError: null });
    expect(useMcpStore.getState().serversByClient).toBe(cached);
    await vi.advanceTimersByTimeAsync(MCP_SCAN_INTERVAL_MS);
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(6);
    job.resolve([server]);
    await useMcpStore.getState().initialize();
    vi.mocked(commands.aiGetMcpServers).mockRejectedValueOnce(new Error("offline"));
    await vi.advanceTimersByTimeAsync(MCP_SCAN_INTERVAL_MS);
    expect(useMcpStore.getState()).toMatchObject({ loadError: null, loading: false });
    expect(useMcpStore.getState().overview?.totalServers).toBe(1);
  });

  it("defers a scheduled scan while an explicit action is writing configuration", async () => {
    await useMcpStore.getState().initialize();
    expect(useMcpStore.getState().beginAction("toggle")).toBe(true);
    await vi.advanceTimersByTimeAsync(MCP_SCAN_INTERVAL_MS);
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(3);
    useMcpStore.getState().finishAction("toggle");
    await useMcpStore.getState().initialize();
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(6);
  });

  it("allows actions during background scans and never commits their stale response", async () => {
    await useMcpStore.getState().initialize();
    const job = deferred<AiMcpServerItem[]>();
    vi.mocked(commands.aiGetMcpServers).mockImplementationOnce(() => job.promise);
    const background = useMcpStore.getState().refresh({ silent: true });
    await Promise.resolve();
    expect(useMcpStore.getState().beginAction("delete")).toBe(true);
    useMcpStore.getState().updateServers((current) => ({ ...current, codex: [] }));
    job.resolve([server]);
    expect(await background).toBe(false);
    expect(useMcpStore.getState().serversByClient.codex).toEqual([]);
    vi.mocked(commands.aiGetMcpServers).mockResolvedValue([]);
    useMcpStore.getState().finishAction("delete");
    await useMcpStore.getState().initialize();
    expect(useMcpStore.getState().serversByClient.codex).toEqual([]);
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(9);
  });

  it("queues a manual refresh behind an automatic scan without parallel reads", async () => {
    await useMcpStore.getState().initialize();
    const job = deferred<AiMcpServerItem[]>();
    vi.mocked(commands.aiGetMcpServers).mockImplementationOnce(() => job.promise);
    const background = useMcpStore.getState().refresh({ silent: true });
    await Promise.resolve();
    expect(useMcpStore.getState().beginAction("refresh")).toBe(true);
    const manual = useMcpStore.getState().refresh();
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(6);
    job.resolve([server]);
    await background;
    expect(await manual).toBe(true);
    useMcpStore.getState().finishAction("refresh");
    expect(commands.aiGetMcpServers).toHaveBeenCalledTimes(9);
    expect(useMcpStore.getState()).toMatchObject({ loading: false, exclusiveAction: null });
  });
});
