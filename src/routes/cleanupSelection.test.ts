import { describe, expect, it } from "vitest";
import type { CleanupCategory, PathDetail } from "@/bindings";
import {
  MAX_PROJECT_ROOTS,
  addProjectRoot,
  buildDisplayCategories,
  cleanupCategorySelection,
  normalizePathKey,
  normalizeProjectRoots,
  removeProjectRoot,
  replaceProjectRoot,
  restoreSelectedCategories,
  scanButtonState,
  selectedCleanupPaths,
  setCategoriesCheckedState,
  togglePathState,
  validExcludedPaths,
} from "@/routes/cleanupSelection";

function path(path: string, sizeBytes = 1): PathDetail {
  return {
    path,
    sizeBytes,
    fileCount: 1,
    matchedRule: "test-rule",
    source: "test",
    volumeSerial: 1,
    fileId: sizeBytes,
  };
}

function category(
  id: string,
  riskLevel: CleanupCategory["riskLevel"],
  defaultSelected: boolean,
  paths: PathDetail[],
): CleanupCategory {
  return {
    id,
    name: id,
    description: id,
    sizeBytes: paths.reduce((sum, item) => sum + item.sizeBytes, 0),
    fileCount: paths.length,
    paths,
    riskLevel,
    defaultSelected,
    minAgeDays: null,
  };
}

describe("cleanup selection", () => {
  it("normalizes extended and UNC Windows paths", () => {
    expect(normalizePathKey('"\\\\?\\C:\\Temp\\"')).toBe("c:\\temp");
    expect(normalizePathKey("\\\\?\\UNC\\server\\share\\Cache")).toBe("\\\\server\\share\\cache");
    expect(normalizePathKey("C:/Temp/cache/")).toBe("c:\\temp\\cache");
  });

  it("selecting one path does not select sibling paths", () => {
    const first = path("C:\\cache\\one");
    const second = path("C:\\cache\\two");
    const item = category("app-cache", "caution", false, [first, second]);
    const next = togglePathState(item, first, new Set(), new Set());

    expect(next.selected).toEqual(new Set([item.id]));
    expect(next.excludedPaths).toEqual(new Set([normalizePathKey(second.path)]));
    expect(cleanupCategorySelection(item, next.selected, next.excludedPaths)).toMatchObject({
      checked: false,
      indeterminate: true,
      checkedPathCount: 1,
    });
  });

  it("unselecting the last path clears its category", () => {
    const only = path("C:\\cache\\only");
    const item = category("win-temp", "safe", true, [only]);
    const next = togglePathState(item, only, new Set([item.id]), new Set());
    expect(next.selected.has(item.id)).toBe(false);
    expect(next.excludedPaths.has(normalizePathKey(only.path))).toBe(true);
  });

  it("bulk safe selection leaves caution and advanced categories unchanged", () => {
    const safe = category("win-temp", "safe", false, [path("C:\\safe")]);
    const caution = category("browser-cache", "caution", false, [path("C:\\caution")]);
    const advanced = category("rust-target", "advanced", false, [path("C:\\advanced")]);
    const next = setCategoriesCheckedState([safe], new Set([advanced.id]), new Set(), true);

    expect(next.selected).toEqual(new Set([safe.id, advanced.id]));
    expect(next.selected.has(caution.id)).toBe(false);
  });

  it("prunes stale exclusions while accepting differently prefixed paths", () => {
    const item = category("win-temp", "safe", true, [path("C:\\Temp\\Cache")]);
    const result = validExcludedPaths(
      [item],
      new Set(["\\\\?\\C:\\Temp\\Cache", "D:\\gone"]),
    );
    expect(result).toEqual(new Set([normalizePathKey(item.paths[0].path)]));
  });

  it("groups programming categories without changing child risk or selection", () => {
    const rust = category("rust-target", "advanced", false, [path("C:\\work\\target", 10)]);
    const dotnet = category("dotnet-cache", "advanced", false, [path("C:\\nuget", 20)]);
    const safe = category("win-temp", "safe", true, [path("C:\\temp", 5)]);
    const display = buildDisplayCategories([safe, rust, dotnet]);
    const programming = display.find((item) => item.id === "programming-cache");

    expect(programming?.sizeBytes).toBe(30);
    expect(programming?.childCategories).toEqual([rust, dotnet]);
    expect(programming?.riskLevel).toBe("advanced");
  });

  it("adds, normalizes and deduplicates project roots", () => {
    expect(addProjectRoot(["C:\\Work\\"], "c:/work")).toEqual(["C:\\Work"]);
    expect(normalizeProjectRoots(['"D:\\Code\\"', "D:/Code", " "])).toEqual(["D:\\Code"]);
  });

  it("replaces and removes project roots while preserving order", () => {
    const roots = ["C:\\One", "D:\\Two", "E:\\Three"];
    expect(replaceProjectRoot(roots, 1, "F:\\Four")).toEqual(["C:\\One", "F:\\Four", "E:\\Three"]);
    expect(removeProjectRoot(roots, 0)).toEqual(["D:\\Two", "E:\\Three"]);
  });

  it("limits project roots to sixteen entries", () => {
    const roots = Array.from({ length: MAX_PROJECT_ROOTS + 4 }, (_, index) => `C:\\project-${index}`);
    expect(normalizeProjectRoots(roots)).toHaveLength(MAX_PROJECT_ROOTS);
  });

  it("restores persisted category selection and prunes missing categories", () => {
    const safe = category("win-temp", "safe", true, [path("C:\\Temp")]);
    const rust = category("rust-target", "advanced", false, [path("C:\\Work\\target")]);
    expect(restoreSelectedCategories([safe, rust], [rust.id, "removed-category"]))
      .toEqual(new Set([rust.id]));
    expect(restoreSelectedCategories([safe, rust], null)).toEqual(new Set([safe.id]));
  });

  it("new paths inherit a selected category unless explicitly excluded", () => {
    const oldPath = path("C:\\cache\\old");
    const newPath = path("\\\\?\\C:\\cache\\new");
    const rescanned = category("win-temp", "safe", true, [oldPath, newPath]);
    const exclusions = validExcludedPaths(
      [rescanned],
      new Set([normalizePathKey(oldPath.path), "D:\\gone"]),
    );
    const state = cleanupCategorySelection(rescanned, new Set([rescanned.id]), exclusions);

    expect(state.checkedPathCount).toBe(1);
    expect(state.indeterminate).toBe(true);
    expect(exclusions.has(normalizePathKey(newPath.path))).toBe(false);
  });

  it("drives the merged scan and stop button state machine", () => {
    expect(scanButtonState({ scanning: false, cancelRequested: false, cleaning: false, exporting: false }))
      .toEqual({ action: "start", label: "开始扫描", danger: false, disabled: false });
    expect(scanButtonState({ scanning: true, cancelRequested: false, cleaning: false, exporting: false }))
      .toEqual({ action: "stop", label: "停止扫描", danger: true, disabled: false });
    expect(scanButtonState({ scanning: true, cancelRequested: true, cleaning: false, exporting: false }))
      .toEqual({ action: "wait", label: "正在停止", danger: true, disabled: true });
    expect(scanButtonState({ scanning: false, cancelRequested: false, cleaning: true, exporting: false }).disabled)
      .toBe(true);
  });

  it("exports only checked category paths", () => {
    const first = path("C:\\cache\\one");
    const second = path("C:\\cache\\two");
    const selectedCategory = category("node-cache", "advanced", false, [first, second]);
    const unselectedCategory = category("rust-target", "advanced", false, [path("C:\\work\\target")]);

    expect(selectedCleanupPaths(
      [selectedCategory, unselectedCategory],
      new Set([selectedCategory.id]),
      new Set([normalizePathKey(second.path)]),
    )).toEqual([{ categoryId: selectedCategory.id, path: first.path }]);
  });
});
