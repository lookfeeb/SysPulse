use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct FileStamp {
    pub(super) len: u64,
    pub(super) modified_nanos: u128,
}

pub(super) fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileStamp {
        len: metadata.len(),
        modified_nanos: metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos(),
    })
}

pub(super) fn file_cache_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[derive(Clone)]
pub(super) struct SummaryCacheEntry {
    stamp: FileStamp,
    summary: Option<ParsedSummary>,
}

pub(super) type SummaryCache = Mutex<HashMap<PathBuf, SummaryCacheEntry>>;

pub(super) fn cached_summary(
    cache: &SummaryCache,
    path: &Path,
    scan: impl FnOnce(&Path) -> Option<ParsedSummary>,
) -> Option<ParsedSummary> {
    let stamp = file_stamp(path)?;
    let key = file_cache_key(path);
    if let Ok(cache) = cache.lock() {
        if let Some(entry) = cache.get(&key).filter(|entry| entry.stamp == stamp) {
            return entry.summary.clone();
        }
    }
    let summary = scan(path);
    // 正文可能由外部进程追加，只有完整稳定扫描的结果才进入缓存。
    if file_stamp(path) == Some(stamp) {
        if let Ok(mut cache) = cache.lock() {
            if cache.len() >= 20_000 && !cache.contains_key(&key) {
                cache.clear();
            }
            cache.insert(
                key,
                SummaryCacheEntry {
                    stamp,
                    summary: summary.clone(),
                },
            );
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_files_skip_parsing_and_appends_invalidate_summary() {
        let path = std::env::temp_dir().join(format!("syspulse-summary-{}", uuid::Uuid::new_v4()));
        fs::write(&path, "first").unwrap();
        let cache = SummaryCache::default();
        cached_summary(&cache, &path, |_| None);
        cached_summary(&cache, &path, |_| panic!("unchanged file parsed again"));
        fs::write(&path, "appended content").unwrap();
        let mut called = false;
        cached_summary(&cache, &path, |_| {
            called = true;
            None
        });
        assert!(called);
        fs::remove_file(path).unwrap();
    }
}
