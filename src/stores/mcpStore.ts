import { create } from "zustand";
import { commands, type AiMcpOAuthStatus, type AiMcpOverview, type AiMcpServerItem } from "@/bindings";
import { CLIENTS, EMPTY_SERVERS, isRemoteType, serverKey, sortMcpServers, type McpClient, type ServerFilter } from "@/components/AiManagement/mcpPresentation";
import { errorMessage } from "@/utils/errorMessage";

type Servers = Record<McpClient, AiMcpServerItem[]>;
type OAuthStatuses = Record<string, AiMcpOAuthStatus>;
interface McpState {
  initialized: boolean;
  loading: boolean;
  loadError: string | null;
  overview: AiMcpOverview | null;
  serversByClient: Servers;
  oauthByServer: OAuthStatuses;
  activeClient: McpClient;
  serverQuery: string;
  serverFilter: ServerFilter;
  busy: Record<string, boolean>;
  exclusiveAction: string | null;
  scanning: boolean;
  initialize: () => Promise<boolean>;
  refresh: (options?: { silent?: boolean }) => Promise<boolean>;
  updateServers: (update: (current: Servers) => Servers) => void;
  syncClient: (client: McpClient) => Promise<void>;
  beginAction: (key: string) => boolean;
  finishAction: (key: string) => void;
}

function summarize(servers: Servers): AiMcpOverview {
  const clients = CLIENTS.map(({ key }) => ({ client: key, totalServers: servers[key].length,
    enabledServers: servers[key].filter((server) => !server.disabled).length }));
  return { clients, totalServers: clients.reduce((sum, client) => sum + client.totalServers, 0),
    enabledServers: clients.reduce((sum, client) => sum + client.enabledServers, 0) };
}

async function readOAuth(servers: AiMcpServerItem[]): Promise<OAuthStatuses> {
  const remote = servers.filter((server) => isRemoteType(server.type));
  const results = await Promise.allSettled(remote.map((server) => commands.aiMcpOauthStatus({ client: server.client, name: server.name })));
  return Object.fromEntries(results.map((result, index) => [serverKey(remote[index]), result.status === "fulfilled"
    ? result.value
    : { oauthSupported: null, authorized: false, expiresAt: 0, expiringSoon: false, expired: false,
      refreshFailed: false, needsReauth: false, message: errorMessage(result.reason) } satisfies AiMcpOAuthStatus]));
}

export const MCP_SCAN_INTERVAL_MS = 2 * 60 * 60 * 1000;
let scanTimer: ReturnType<typeof setInterval> | undefined;
let refreshTask: { promise: Promise<boolean>; silent: boolean } | null = null;
let revision = 0;
let pendingScan = false;

function resumePendingScan() {
  if (pendingScan && !refreshTask && !useMcpStore.getState().exclusiveAction) {
    void useMcpStore.getState().refresh({ silent: true });
  }
}

if (import.meta.hot) import.meta.hot.dispose(() => clearInterval(scanTimer));

// Session-lifetime cache. Reading the store or mounting the MCP page never triggers I/O.
export const useMcpStore = create<McpState>((set, get) => ({
  initialized: false, loading: false, loadError: null, overview: null,
  serversByClient: EMPTY_SERVERS, oauthByServer: {}, activeClient: "codex", serverQuery: "", serverFilter: "all",
  busy: {}, exclusiveAction: null, scanning: false,
  initialize: () => {
    if (get().initialized) return refreshTask?.promise ?? Promise.resolve(get().overview !== null);
    clearInterval(scanTimer);
    pendingScan = false;
    set({ initialized: true });
    // The cadence belongs to the app lifetime; manual refreshes and route changes do not reset it.
    // Native configuration reads already use run_blocking_task / spawn_blocking.
    scanTimer = setInterval(() => { void get().refresh({ silent: true }); }, MCP_SCAN_INTERVAL_MS);
    return get().refresh({ silent: true });
  },
  refresh: ({ silent = false } = {}) => {
    if (get().exclusiveAction && (silent || get().exclusiveAction !== "refresh")) {
      if (silent) pendingScan = true;
      return Promise.resolve(false);
    }
    if (refreshTask) {
      // A manual refresh waits for the background reader, then reads after any user mutation.
      if (!silent && refreshTask.silent) return refreshTask.promise.then(() => get().refresh());
      return refreshTask.promise;
    }
    pendingScan = false;
    const startedRevision = revision;
    if (!silent) set({ loading: true, loadError: null });
    const promise = Promise.resolve().then(async () => {
      try {
        const results = await Promise.allSettled(CLIENTS.map(({ key }) => commands.aiGetMcpServers({ client: key })));
        const next = { ...EMPTY_SERVERS };
        for (let index = 0; index < results.length; index++) {
          const result = results[index];
          if (result.status === "rejected") throw result.reason;
          next[CLIENTS[index].key] = sortMcpServers(result.value);
        }
        const oauth = await readOAuth(Object.values(next).flat());
        if (startedRevision !== revision) {
          pendingScan = true;
          return false;
        }
        set({ serversByClient: next, overview: summarize(next), oauthByServer: oauth, loadError: null });
        return true;
      } catch (error) {
        // Automatic scans preserve the visible snapshot and stay silent on failure.
        if (!silent) set({ loadError: errorMessage(error) });
        return false;
      } finally {
        if (!silent) set({ loading: false });
      }
    }).finally(() => { refreshTask = null; resumePendingScan(); });
    refreshTask = { promise, silent };
    return promise;
  },
  updateServers: (update) => set((state) => {
    revision++;
    const servers = update(state.serversByClient);
    const keys = new Set(Object.values(servers).flat().filter((server) => isRemoteType(server.type)).map(serverKey));
    return { serversByClient: servers, overview: summarize(servers),
      oauthByServer: Object.fromEntries(Object.entries(state.oauthByServer).filter(([key]) => keys.has(key))) };
  }),
  // Explicit import/copy/OAuth operations can rewrite canonical client configuration.
  // Reconcile only that client, without reloading the other clients or the overview.
  syncClient: async (client) => {
    revision++;
    const servers = sortMcpServers(await commands.aiGetMcpServers({ client }));
    const oauth = await readOAuth(servers);
    get().updateServers((current) => ({ ...current, [client]: servers }));
    set((state) => ({ oauthByServer: { ...state.oauthByServer, ...oauth } }));
  },
  beginAction: (key) => {
    if (get().loading || get().exclusiveAction !== null) return false;
    revision++;
    set({ exclusiveAction: key });
    return true;
  },
  finishAction: (key) => {
    if (get().exclusiveAction === key) {
      revision++;
      set({ exclusiveAction: null });
      resumePendingScan();
    }
  },
}));
