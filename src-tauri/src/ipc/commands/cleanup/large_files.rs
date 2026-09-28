use super::*;

// ─── Large file scan ────────────────────────────────────────────────────────

pub(super) fn do_scan_large_files(args: &LargeFileScanArgs) -> LargeFileScanResult {
    let min_bytes = args.min_size_mb * 1024 * 1024;
    let limit = args.limit.min(500) as usize;
    let mut files: Vec<LargeFile> = Vec::new();
    let mut total_scanned = 0u64;

    let root = Path::new(&args.root);
    if !root.exists() {
        return LargeFileScanResult {
            files,
            total_scanned: 0,
        };
    }

    scan_large_recursive(
        root,
        min_bytes,
        limit,
        &mut files,
        &mut total_scanned,
        0,
        20,
    );
    files.sort_by_key(|file| std::cmp::Reverse(file.size_bytes));
    files.truncate(limit);

    LargeFileScanResult {
        files,
        total_scanned,
    }
}

pub(super) fn scan_large_recursive(
    dir: &Path,
    min_bytes: u64,
    limit: usize,
    results: &mut Vec<LargeFile>,
    total_scanned: &mut u64,
    depth: u32,
    max_depth: u32,
) {
    if depth > max_depth {
        return;
    }
    if results.len() >= limit.saturating_mul(4).max(limit) {
        compact_large_file_results(results, limit);
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let path = entry.path();
        if meta.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Skip system dirs that are slow/inaccessible
            if name_str.starts_with('$') || name_str == "System Volume Information" {
                continue;
            }
            scan_large_recursive(
                &path,
                min_bytes,
                limit,
                results,
                total_scanned,
                depth + 1,
                max_depth,
            );
        } else {
            *total_scanned += 1;
            let size = meta.len();
            if size >= min_bytes {
                results.push(LargeFile {
                    path: path.to_string_lossy().to_string(),
                    size_bytes: size,
                });
                if results.len() >= limit.saturating_mul(4).max(limit) {
                    compact_large_file_results(results, limit);
                }
            }
        }
    }
}

pub(super) fn compact_large_file_results(results: &mut Vec<LargeFile>, limit: usize) {
    if limit == 0 {
        results.clear();
        return;
    }
    results.sort_by_key(|file| std::cmp::Reverse(file.size_bytes));
    results.truncate(limit);
}

// ─── Helpers ────────────────────────────────────────────────────────────────
