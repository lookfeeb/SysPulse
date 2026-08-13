import type { CleanupCategory, PathDetail } from "@/bindings";

export const PROGRAMMING_CATEGORY_ID = "programming-cache";
export const PROGRAMMING_CATEGORY_IDS = new Set([
  "rust-target",
  "go-cache",
  "python-cache",
  "node-cache",
  "cpp-cache",
  "dotnet-cache",
  "java-cache",
]);

export type DisplayCategory = CleanupCategory & {
  childCategories?: CleanupCategory[];
};

export type SelectionState = {
  checked: boolean;
  indeterminate: boolean;
  checkedPathCount: number;
  totalPathCount: number;
  selectedCategoryCount: number;
  totalCategoryCount: number;
};

export const MAX_PROJECT_ROOTS = 16;

export type ScanButtonState = {
  action: "start" | "stop" | "wait";
  label: "开始扫描" | "停止扫描" | "正在停止";
  danger: boolean;
  disabled: boolean;
};

export function scanButtonState({
  scanning,
  cancelRequested,
  cleaning,
  exporting,
}: {
  scanning: boolean;
  cancelRequested: boolean;
  cleaning: boolean;
  exporting: boolean;
}): ScanButtonState {
  if (cancelRequested) {
    return { action: "wait", label: "正在停止", danger: true, disabled: true };
  }
  if (scanning) {
    return { action: "stop", label: "停止扫描", danger: true, disabled: false };
  }
  return {
    action: "start",
    label: "开始扫描",
    danger: false,
    disabled: cleaning || exporting,
  };
}

export function restoreSelectedCategories(
  categories: CleanupCategory[],
  storedCategoryIds: Iterable<string> | null,
): Set<string> {
  if (storedCategoryIds === null) {
    return new Set(
      categories
        .filter((category) => category.defaultSelected)
        .map((category) => category.id),
    );
  }
  const availableIds = new Set(categories.map((category) => category.id));
  return new Set([...storedCategoryIds].filter((id) => availableIds.has(id)));
}

export function normalizePathKey(path: string): string {
  let normalized = path.trim().replace(/^"|"$/g, "").replaceAll("/", "\\").replace(/\\+$/, "").toLowerCase();
  if (normalized.startsWith("\\\\?\\unc\\")) normalized = `\\\\${normalized.slice(8)}`;
  else if (normalized.startsWith("\\\\?\\")) normalized = normalized.slice(4);
  return normalized;
}

export function normalizeProjectRoots(roots: Iterable<string>): string[] {
  const result: string[] = [];
  const seen = new Set<string>();
  for (const value of roots) {
    const path = value.trim().replace(/^"|"$/g, "").replace(/[\\/]+$/, "");
    const key = normalizePathKey(path);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    result.push(path);
    if (result.length >= MAX_PROJECT_ROOTS) break;
  }
  return result;
}

export function addProjectRoot(roots: Iterable<string>, root: string): string[] {
  return normalizeProjectRoots([...roots, root]);
}

export function replaceProjectRoot(roots: Iterable<string>, index: number, root: string): string[] {
  const current = normalizeProjectRoots(roots);
  if (index < 0 || index >= current.length) return current;
  current[index] = root;
  return normalizeProjectRoots(current);
}

export function removeProjectRoot(roots: Iterable<string>, index: number): string[] {
  return normalizeProjectRoots(roots).filter((_, itemIndex) => itemIndex !== index);
}

export function validExcludedPaths(categories: CleanupCategory[], excludedPaths: Set<string>): Set<string> {
  const available = new Set(categories.flatMap((category) => category.paths.map((path) => normalizePathKey(path.path))));
  return new Set(
    [...excludedPaths]
      .map(normalizePathKey)
      .filter((path) => available.has(path)),
  );
}

export function buildDisplayCategories(categories: CleanupCategory[]): DisplayCategory[] {
  const programmingCategories = categories.filter((cat) => PROGRAMMING_CATEGORY_IDS.has(cat.id));
  const otherCategories = categories.filter((cat) => !PROGRAMMING_CATEGORY_IDS.has(cat.id));

  if (programmingCategories.length === 0) return otherCategories;

  return [
    ...otherCategories,
    {
      id: PROGRAMMING_CATEGORY_ID,
      name: "编程缓存",
      description: programmingCategories.map((cat) => cat.name.replace(/\s*缓存$/, "")).join(" / "),
      sizeBytes: programmingCategories.reduce((sum, cat) => sum + cat.sizeBytes, 0),
      fileCount: programmingCategories.reduce((sum, cat) => sum + cat.fileCount, 0),
      paths: programmingCategories.flatMap((cat) => cat.paths),
      riskLevel: "advanced",
      defaultSelected: false,
      minAgeDays: null,
      childCategories: programmingCategories,
    },
  ];
}

export function checkedPaths(paths: PathDetail[], excludedPaths: Set<string>): PathDetail[] {
  return paths.filter((path) => !excludedPaths.has(normalizePathKey(path.path)));
}

export function sumPathSize(paths: PathDetail[]): number {
  return paths.reduce((sum, path) => sum + path.sizeBytes, 0);
}

export function sumPathFiles(paths: PathDetail[]): number {
  return paths.reduce((sum, path) => sum + path.fileCount, 0);
}

export function cleanablePaths(category: CleanupCategory, selected: Set<string>, excludedPaths: Set<string>): PathDetail[] {
  if (!selected.has(category.id)) return [];
  return checkedPaths(category.paths, excludedPaths);
}

export function selectedCleanupPaths(
  categories: CleanupCategory[],
  selected: Set<string>,
  excludedPaths: Set<string>,
): Array<{ categoryId: string; path: string }> {
  return categories.flatMap((category) =>
    cleanablePaths(category, selected, excludedPaths).map((detail) => ({
      categoryId: category.id,
      path: detail.path,
    })),
  );
}

export function cleanupCategorySelection(
  category: CleanupCategory,
  selected: Set<string>,
  excludedPaths: Set<string>,
): SelectionState {
  const categorySelected = selected.has(category.id);
  const totalPathCount = category.paths.length;
  const checkedPathCount = categorySelected ? checkedPaths(category.paths, excludedPaths).length : 0;

  return {
    checked: categorySelected && (totalPathCount === 0 || checkedPathCount === totalPathCount),
    indeterminate: categorySelected && checkedPathCount > 0 && checkedPathCount < totalPathCount,
    checkedPathCount,
    totalPathCount,
    selectedCategoryCount: categorySelected ? 1 : 0,
    totalCategoryCount: 1,
  };
}

export function displayCategorySelection(
  category: DisplayCategory,
  selected: Set<string>,
  excludedPaths: Set<string>,
): SelectionState {
  const childStates = (category.childCategories ?? [category]).map((child) =>
    cleanupCategorySelection(child, selected, excludedPaths),
  );
  const checked = childStates.length > 0 && childStates.every((state) => state.checked);
  const hasSelection = childStates.some((state) => state.checked || state.indeterminate);

  return {
    checked,
    indeterminate: hasSelection && !checked,
    checkedPathCount: childStates.reduce((sum, state) => sum + state.checkedPathCount, 0),
    totalPathCount: childStates.reduce((sum, state) => sum + state.totalPathCount, 0),
    selectedCategoryCount: childStates.reduce((sum, state) => sum + state.selectedCategoryCount, 0),
    totalCategoryCount: childStates.reduce((sum, state) => sum + state.totalCategoryCount, 0),
  };
}

export function setCategoriesCheckedState(
  categories: CleanupCategory[],
  selected: Set<string>,
  excludedPaths: Set<string>,
  checked: boolean,
): { selected: Set<string>; excludedPaths: Set<string> } {
  const nextSelected = new Set(selected);
  const nextExcludedPaths = new Set(excludedPaths);
  categories.forEach((category) => {
    if (checked) nextSelected.add(category.id);
    else nextSelected.delete(category.id);
    category.paths.forEach((path) => {
      const key = normalizePathKey(path.path);
      if (checked) nextExcludedPaths.delete(key);
      else nextExcludedPaths.add(key);
    });
  });
  return { selected: nextSelected, excludedPaths: nextExcludedPaths };
}

export function togglePathState(
  category: CleanupCategory,
  path: PathDetail,
  selected: Set<string>,
  excludedPaths: Set<string>,
): { selected: Set<string>; excludedPaths: Set<string> } {
  const nextSelected = new Set(selected);
  const nextExcludedPaths = new Set(excludedPaths);
  const pathKey = normalizePathKey(path.path);
  const pathChecked = selected.has(category.id) && !excludedPaths.has(pathKey);

  if (pathChecked) {
    nextExcludedPaths.add(pathKey);
    const hasCheckedPath = category.paths.some((item) =>
      normalizePathKey(item.path) !== pathKey && !nextExcludedPaths.has(normalizePathKey(item.path)),
    );
    if (!hasCheckedPath) nextSelected.delete(category.id);
  } else {
    if (!selected.has(category.id)) {
      category.paths.forEach((item) => nextExcludedPaths.add(normalizePathKey(item.path)));
    }
    nextSelected.add(category.id);
    nextExcludedPaths.delete(pathKey);
  }

  return { selected: nextSelected, excludedPaths: nextExcludedPaths };
}
