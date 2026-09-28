use super::oauth_store::McpOAuthStore;
use parking_lot::RwLock;
use std::sync::Arc;

/// 单实例应用内的只读快照；写入必须先持久化成功，再发布给请求处理器。
#[derive(Default)]
pub(super) struct OAuthCache(RwLock<Option<Arc<McpOAuthStore>>>);

impl OAuthCache {
    pub(super) fn get(&self) -> Option<Arc<McpOAuthStore>> {
        self.0.read().clone()
    }

    pub(super) fn publish(&self, store: McpOAuthStore) {
        *self.0.write() = Some(Arc::new(store));
    }

    pub(super) fn commit(
        &self,
        store: &McpOAuthStore,
        persist: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        persist()?;
        self.publish(store.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_reuse_snapshot_and_failed_writes_do_not_publish() {
        let cache = OAuthCache::default();
        let mut initial = McpOAuthStore::default();
        initial
            .server_bindings
            .insert("client:server".into(), "credential".into());
        cache.commit(&initial, || Ok(())).unwrap();
        let before = cache.get().unwrap();
        assert!(Arc::ptr_eq(&before, &cache.get().unwrap()));
        let revoked = McpOAuthStore::default();
        assert!(cache
            .commit(&revoked, || Err("database locked".into()))
            .is_err());
        assert!(Arc::ptr_eq(&before, &cache.get().unwrap()));
        cache.commit(&revoked, || Ok(())).unwrap();
        assert!(cache.get().unwrap().server_bindings.is_empty());
        assert_eq!(before.server_bindings.len(), 1);
    }
}
