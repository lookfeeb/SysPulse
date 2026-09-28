import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppConfig } from "@/bindings";
import * as configApi from "@/ipc";
import { useConfigStore } from "./configStore";

const config = { general: { sampleIntervalMs: 1000 } } as AppConfig;

describe("config loading diagnostics", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useConfigStore.setState(useConfigStore.getInitialState());
  });

  it("preserves the first failure through further failures and successful retry", async () => {
    const getConfig = vi.spyOn(configApi, "getConfig")
      .mockRejectedValueOnce(new configApi.IpcError("NOT_READY", "first failure"))
      .mockRejectedValueOnce(new Error("second failure"))
      .mockResolvedValueOnce(config);

    await useConfigStore.getState().load();
    const first = useConfigStore.getState().firstLoadFailure;
    expect(first).toContain("get_config [NOT_READY] first failure");
    expect(useConfigStore.getState()).toMatchObject({ config: null, loading: false, error: "first failure" });

    await useConfigStore.getState().load();
    expect(useConfigStore.getState().firstLoadFailure).toBe(first);
    expect(useConfigStore.getState().error).toBe("second failure");

    await useConfigStore.getState().load();
    expect(useConfigStore.getState()).toMatchObject({ config, loading: false, error: null, firstLoadFailure: first });
    expect(getConfig).toHaveBeenCalledTimes(3);
  });

  it("does not create diagnostics for a successful initial load", async () => {
    vi.spyOn(configApi, "getConfig").mockResolvedValue(config);
    await useConfigStore.getState().load();
    expect(useConfigStore.getState()).toMatchObject({ config, firstLoadFailure: null, error: null });
  });
});
