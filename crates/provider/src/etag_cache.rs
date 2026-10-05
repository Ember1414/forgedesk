//! GET 请求的 ETag 缓存（T4.10：限流降级的数据来源）。
//!
//! # 为什么在 HTTP 底座做而不到各服务
//!
//! ETag 的写入口只有一个（[`crate::client::GitHubHttp::send`]），缓存放
//! 在那里对所有 GET 透明生效：304 直接回放缓存体（省配额），限流/断网
//! 时回退"过期但可用"的缓存体（ARCHITECTURE.md §4.3 的降级路径）。
//! 各服务方法零改动，也不会出现"某个列表忘了接缓存"的漏网之鱼。
//!
//! # 缓存什么
//!
//! 只缓存**小的 JSON 响应**（≤ [`MAX_CACHED_BODY_BYTES`]）：列表/详情
//! 的形态。大响应（日志流、raw README）与二进制不缓存——它们的价值
//! 不在"降级时保命"，而缓存它们会把内存吃穿。
//!
//! # 一致性语义
//!
//! 条目**永不过期**：304 校验由 GitHub 决定数据是否变化，缓存的 etag
//! 一直有效；降级回退是"明知的过期"——调用方拿到的是上次成功响应的
//! 内容，由 UI 的限流横幅告知用户"显示的是缓存数据"。

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// 单条缓存体上限（1MB）：列表/详情绰绰有余，日志流/大文件进不来。
pub const MAX_CACHED_BODY_BYTES: usize = 1024 * 1024;

/// 条目数上限：LRU 语义（超出淘汰最早写入的）。
const MAX_ENTRIES: usize = 64;

#[derive(Debug, Clone)]
struct CacheEntry {
    etag: String,
    content_type: String,
    body: Vec<u8>,
}

/// 线程安全的 ETag 缓存（Arc 共享：GitHubHttp 克隆之间同一份）。
#[derive(Debug, Clone, Default)]
pub struct EtagCache {
    entries: Arc<RwLock<HashMap<String, CacheEntry>>>,
    // 插入顺序（淘汰最早写入的条目）；与 entries 里的 key 一一对应
    order: Arc<RwLock<std::collections::VecDeque<String>>>,
}

impl EtagCache {
    /// 新建空缓存。
    pub fn new() -> Self {
        Self::default()
    }

    /// 该请求（上一轮）拿到的 etag；没有条目为 `None`。
    #[must_use]
    pub fn etag_for(&self, key: &str) -> Option<String> {
        self.entries.read().get(key).map(|e| e.etag.clone())
    }

    /// 该请求的缓存体（降级回退用）。
    #[must_use]
    pub fn body_for(&self, key: &str) -> Option<(String, Vec<u8>)> {
        self.entries
            .read()
            .get(key)
            .map(|e| (e.content_type.clone(), e.body.clone()))
    }

    /// 写入一条（etags/响应体从上一轮成功响应取得）。
    ///
    /// 体为空或超过 [`MAX_CACHED_BODY_BYTES`] 时静默跳过：这种响应没有
    /// 缓存价值（日志流），不值得占位。
    pub fn store(&self, key: &str, etag: &str, content_type: &str, body: Vec<u8>) {
        if body.is_empty() || body.len() > MAX_CACHED_BODY_BYTES {
            return;
        }
        let mut entries = self.entries.write();
        if !entries.contains_key(key) {
            let mut order = self.order.write();
            order.push_back(key.to_owned());
            while order.len() > MAX_ENTRIES {
                if let Some(oldest) = order.pop_front() {
                    entries.remove(&oldest);
                }
            }
        }
        entries.insert(
            key.to_owned(),
            CacheEntry {
                etag: etag.to_owned(),
                content_type: content_type.to_owned(),
                body,
            },
        );
    }

    /// 清空（测试与"彻底刷新"场景）。
    pub fn clear(&self) {
        self.entries.write().clear();
        self.order.write().clear();
    }
}

/// 组装缓存键：URL + 查询串（分页翻页各有各的缓存）。
pub(crate) fn cache_key(url: &str, query: Option<&[(String, String)]>) -> String {
    match query {
        None => url.to_owned(),
        Some(pairs) => {
            let mut key = String::with_capacity(url.len() + 32);
            key.push_str(url);
            for (name, value) in pairs {
                key.push('&');
                key.push_str(name);
                key.push('=');
                key.push_str(value);
            }
            key
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{cache_key, EtagCache, MAX_CACHED_BODY_BYTES};

    #[test]
    fn store_and_lookup_round_trip() {
        let cache = EtagCache::new();
        assert_eq!(cache.etag_for("k"), None);
        assert_eq!(cache.body_for("k"), None);

        cache.store("k", "\"v1\"", "application/json", b"{}".to_vec());
        assert_eq!(cache.etag_for("k").as_deref(), Some("\"v1\""));
        let (ct, body) = cache.body_for("k").unwrap();
        assert_eq!(ct, "application/json");
        assert_eq!(body, b"{}");
    }

    #[test]
    fn oversized_and_empty_bodies_are_not_stored() {
        let cache = EtagCache::new();
        cache.store(
            "big",
            "e",
            "application/json",
            vec![0u8; MAX_CACHED_BODY_BYTES + 1],
        );
        cache.store("empty", "e", "application/json", Vec::new());
        assert_eq!(cache.etag_for("big"), None);
        assert_eq!(cache.etag_for("empty"), None);
    }

    #[test]
    fn evicts_the_oldest_entry_beyond_the_cap() {
        let cache = EtagCache::new();
        for i in 0..70 {
            cache.store(&format!("k{i}"), "e", "application/json", b"x".to_vec());
        }
        // 最早的 6 个被淘汰，70-6=64 条存活
        assert_eq!(cache.etag_for("k0"), None);
        assert_eq!(cache.etag_for("k6").as_deref(), Some("e"));
        assert_eq!(cache.etag_for("k69").as_deref(), Some("e"));

        // 已有 key 的重写不改变淘汰顺序、不挤出别的条目
        cache.store("k69", "e2", "application/json", b"y".to_vec());
        assert_eq!(cache.etag_for("k69").as_deref(), Some("e2"));
    }

    #[test]
    fn cache_key_distinguishes_query_strings() {
        let query = vec![("page".to_owned(), "2".to_owned())];
        assert_eq!(cache_key("http://x/a", None), "http://x/a");
        assert_ne!(cache_key("http://x/a", Some(&query)), "http://x/a");
        assert_ne!(
            cache_key("http://x/a", Some(&query)),
            cache_key("http://x/a", Some(&[("page".to_owned(), "3".to_owned())]))
        );
    }
}
