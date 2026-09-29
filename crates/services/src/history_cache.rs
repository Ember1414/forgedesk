//! 日志分页缓存（T2.9）：把"深分页 / 重复首页"的重扫压成 O(页)。
//!
//! # 为什么缓存 walk 前缀而不是布局结果
//!
//! 实测（`docs/PERF-BASELINE.md` §5，10 万提交夹具）：一页 200 行的布局
//! 不到 0.2ms，而 libgit2 取同一页要 400ms 以上——成本在 **walk 与提交对象
//! 水合**，不在布局。因此缓存的是 walk 的**累积提交前缀**（第 0..N 行），
//! 布局仍按页现算：它依赖"折叠窗口是否完整"这类请求期信息，且本身足够便宜。
//!
//! # 键与失效
//!
//! 键 = (repo_id, 查询形状, tips 指纹)：
//!
//! - 查询形状 = `HistoryQuery` 去掉 `cursor` / `page_size`。页大小只影响
//!   每次取多少，不影响 walk 的内容；游标更只是"从缓存的哪一段切"。
//! - tips 指纹 = 分支/标签的 (名字, oid) 有序对 + HEAD oid。**外部 git 进程**
//!   改动仓库也会在下一次指纹计算时被发现——失效不依赖事件通知，宁可
//!   多算一次引用枚举（毫秒级）也不能给出过期历史。
//! - `my_commits_only` 在服务层已翻译成 author 过滤，缓存只见到翻译后的形状。
//!
//! 刻意**不含** `refs/stash`：stash 提交不在任何分支 tip 上，`git log --all`
//! 与 libgit2 的 walk 都不包含它（stash 面板有自己的数据通路）。
//!
//! # 内存上限与绕过
//!
//! 单条目前缀最多 [`MAX_ENTRY_COMMITS`] 条提交，全部条目合计不超过
//! [`MAX_TOTAL_COMMITS`]；游标越过缓存能力时**绕过缓存直连引擎**——深翻到
//! 前缀之外的请求回到逐页 walk 的旧行为，正确性不受影响，只是不再加速。
//! LRU 条目数上限 [`MAX_ENTRIES`]（任务书：LRU，上限 50）。
//!
//! # 并发
//!
//! 命令层把重查询挪出主线程后（`#[tauri::command(async)]`），两个线程可能
//! 同时翻同一份历史：条目本身在 `Arc<Mutex<…>>` 里，扩展串行化在**同键**上；
//! 全局表（LRU 簿记）的锁只在取条目/驱逐时短暂持有，walk 不发生在表锁内。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use forgedesk_domain::git::Commit;

/// 单条目前缀的提交数上限（约 10–20 MB 量级：每条提交是 oid、父、作者与摘要）。
pub const MAX_ENTRY_COMMITS: usize = 20_000;

/// 全部条目的提交总数上限（50 条目 × 2 万条的病态上界远超进程预算，必须另有总闸）。
pub const MAX_TOTAL_COMMITS: usize = 100_000;

/// 锁住互斥量；毒化（持有锁的线程在更新中途 panic）对缓存场景只是
/// "数据可能停在中间态"，比把整个历史查询打死危害小——拿回内部数据继续用。
pub(crate) fn lock_ignoring_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// LRU 条目数上限（T2.9 任务书：LRU，上限 50）。
pub const MAX_ENTRIES: usize = 50;

/// 查询形状键：`HistoryQuery` 去掉分页两兄弟（cursor / page_size）后的全部字段。
///
/// 用**结构化字段**而不是哈希值做键：64 位哈希的碰撞会把 A 查询的历史喂给
/// B 查询，那是静默的数据错误；结构化键的"相等"由 PartialEq 精确保证。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogQueryKey {
    /// 起始引用（与 `HistoryQuery.revision` 一致）。
    pub revision: Option<String>,
    /// 是否包含所有引用。
    pub all_branches: bool,
    /// 路径过滤。
    pub paths: Vec<forgedesk_domain::git::RepoPath>,
    /// 作者过滤（服务层已把 `my_commits_only` 翻译进来）。
    pub author: Option<String>,
    /// 时间下界。
    pub since: Option<i64>,
    /// 时间上界。
    pub until: Option<i64>,
    /// 提交信息子串。
    pub message_contains: Option<String>,
    /// 只看第一父链。
    pub first_parent_only: bool,
    /// 跟随重命名。
    pub follow_renames: bool,
    /// 分支多选。
    pub revisions: Vec<String>,
    /// 忽略大小写。
    pub case_insensitive: bool,
    /// 只看合并提交。
    pub merges_only: bool,
}

/// tips 指纹：分支/标签的 (名字, oid) 有序对 + HEAD oid。
///
/// 排序后比较：引用枚举的顺序是引擎实现细节，不参与"变了没有"的判定。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TipsFingerprint {
    /// 已排序的 (引用名, 提交 oid) 对（分支与标签合并成一张表）。
    pub refs: Vec<(String, String)>,
    /// HEAD 直接指向的 oid（游离 HEAD 时也有值）。
    pub head: Option<String>,
}

/// 缓存键。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    repo_id: i64,
    query: LogQueryKey,
    tips: TipsFingerprint,
}

/// 一份累积的 walk 前缀。
///
/// `commits[i]` 是该查询形状下全历史的第 i 行（新 → 旧）；`exhausted` 表示
/// 前缀已经覆盖到历史末端。字段公开给 [`HistoryService`](crate::HistoryService)：
/// 扩展（走引擎）与切片（服务页面）都在服务层编排，这里只做存储与上限标记。
#[derive(Debug, Default)]
pub struct CachedWalk {
    /// 累积的提交前缀（新 → 旧）。
    pub commits: Vec<Commit>,
    /// 前缀已覆盖到历史末端（引擎返回了不满页的批次）。
    pub exhausted: bool,
}

/// 日志分页缓存。线程安全；表锁只在取条目/驱逐/记账时短暂持有，
/// walk 不发生在表锁内。
#[derive(Debug, Default)]
pub struct LogPageCache {
    inner: Mutex<Inner>,
    /// 单条目前缀的提交数上限。默认 [`MAX_ENTRY_COMMITS`]；测试用小上限
    /// 来驱动"越过上限绕过缓存"的路径，不必生成两万条提交。
    entry_cap: Option<usize>,
    /// 全部条目合计的提交数上限。默认 [`MAX_TOTAL_COMMITS`]；与 `entry_cap`
    /// 同一理由给测试留旋钮。
    total_cap: Option<usize>,
}

#[derive(Debug, Default)]
struct Inner {
    entries: HashMap<CacheKey, Arc<Mutex<CachedWalk>>>,
    /// LRU 序（队首 = 最近使用）。
    order: VecDeque<CacheKey>,
    total_commits: usize,
}

impl LogPageCache {
    /// 建空缓存（单条目上限为 [`MAX_ENTRY_COMMITS`]）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 覆盖单条目上限（下限 1）。
    ///
    /// 生产代码应当用默认值：调小它只会让深翻更早退回直连引擎的旧行为；
    /// 这个旋钮的存在是为了让测试能在几十条提交的夹具上驱动绕过路径。
    pub fn with_entry_cap(mut self, cap: usize) -> Self {
        self.entry_cap = Some(cap.max(1));
        self
    }

    /// 单条目上限。
    pub fn entry_cap(&self) -> usize {
        self.entry_cap.unwrap_or(MAX_ENTRY_COMMITS)
    }

    /// 覆盖全部条目合计的提交数上限（下限 1；与 [`Self::with_entry_cap`]
    /// 同一模式，供测试在小夹具上驱动总闸驱逐）。
    pub fn with_total_cap(mut self, cap: usize) -> Self {
        self.total_cap = Some(cap.max(1));
        self
    }

    /// 全部条目合计的提交数上限。
    fn total_cap(&self) -> usize {
        self.total_cap.unwrap_or(MAX_TOTAL_COMMITS)
    }

    /// 取（或建）键对应的条目并把它标记为最近使用。
    ///
    /// 返回的 `Arc<Mutex<CachedWalk>>` 允许调用方在**表锁之外**扩展与读取；
    /// 被驱逐的条目通过 `Arc` 还活在调用方手里，只是不再被后续请求共享。
    pub fn entry(
        &self,
        repo_id: i64,
        query: LogQueryKey,
        tips: TipsFingerprint,
    ) -> Arc<Mutex<CachedWalk>> {
        let key = CacheKey {
            repo_id,
            query,
            tips,
        };
        let mut inner = self.lock_inner();
        if let Some(existing) = inner.entries.get(&key) {
            let existing = Arc::clone(existing);
            inner.touch(&key);
            return existing;
        }
        let entry = Arc::new(Mutex::new(CachedWalk::default()));
        inner.entries.insert(key.clone(), Arc::clone(&entry));
        inner.order.push_front(key);
        self.evict(&mut inner);
        entry
    }

    /// 当前条目数（测试与诊断用）。
    pub fn len(&self) -> usize {
        self.lock_inner().entries.len()
    }

    /// 是否为空（测试与诊断用）。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 扩展方把新提交追加进条目后调用：更新总闸账目，超出时按 LRU 驱逐。
    ///
    /// 被驱逐的条目可能仍被在途请求通过 `Arc` 持有并继续扩展——那部分提交
    /// 不再计入总闸（轻微低估可接受：总闸是预算护栏，不是精确记账）。
    pub fn record_added(&self, added: usize) {
        let mut inner = self.lock_inner();
        inner.total_commits = inner.total_commits.saturating_add(added);
        if inner.total_commits > self.total_cap() {
            self.evict(&mut inner);
        }
    }

    /// 锁住全局表（毒化处理见 [`lock_ignoring_poison`]）。
    fn lock_inner(&self) -> MutexGuard<'_, Inner> {
        lock_ignoring_poison(&self.inner)
    }

    /// 驱逐超出上限的条目（LRU 从队尾开始）。
    ///
    /// 条目数上限是任务书常量 [`MAX_ENTRIES`]；提交总闸用**实例**的
    /// `total_cap`（测试旋钮必须真的生效）。
    fn evict(&self, inner: &mut Inner) {
        while inner.entries.len() > MAX_ENTRIES || inner.total_commits > self.total_cap() {
            let Some(victim) = inner.order.pop_back() else {
                break;
            };
            if let Some(entry) = inner.entries.remove(&victim) {
                // 条目可能还被在途请求通过 Arc 持有并继续扩展——那部分提交
                // 不再计入总闸（轻微低估可接受：总闸是预算护栏，不是精确记账）
                if let Ok(walk) = entry.lock() {
                    inner.total_commits = inner.total_commits.saturating_sub(walk.commits.len());
                }
            }
        }
    }
}

impl Inner {
    /// 把键移到 LRU 队首。
    fn touch(&mut self, key: &CacheKey) {
        if let Some(position) = self.order.iter().position(|item| item == key) {
            self.order.remove(position);
            self.order.push_front(key.clone());
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{lock_ignoring_poison, LogPageCache, LogQueryKey, TipsFingerprint, MAX_ENTRIES};
    use forgedesk_domain::git::{Commit, Signature, SignatureStatus};

    fn key_with_author(author: &str) -> LogQueryKey {
        LogQueryKey {
            revision: None,
            all_branches: false,
            paths: Vec::new(),
            author: Some(author.to_owned()),
            since: None,
            until: None,
            message_contains: None,
            first_parent_only: false,
            follow_renames: false,
            revisions: Vec::new(),
            case_insensitive: false,
            merges_only: false,
        }
    }

    fn fingerprint_with_head(head: &str) -> TipsFingerprint {
        TipsFingerprint {
            refs: Vec::new(),
            head: Some(head.to_owned()),
        }
    }

    /// 组装一条测试用提交：内容无所谓，够填满账目就行。
    fn fake_commit(index: usize) -> Commit {
        Commit {
            oid: format!("{index:040x}"),
            parents: Vec::new(),
            author: Signature {
                name: "t".into(),
                email: "t@example.invalid".into(),
                time: Some(0),
            },
            committer: Signature {
                name: "t".into(),
                email: "t@example.invalid".into(),
                time: Some(0),
            },
            refs: Vec::new(),
            signature: SignatureStatus::Unsigned,
            subject: format!("commit {index}"),
            body: None,
        }
    }

    /// 建一个带 `count` 条假提交前缀的条目（模拟已被 walk 过的形状）。
    fn fill(cache: &LogPageCache, author: &str, count: usize) {
        let entry = cache.entry(1, key_with_author(author), fingerprint_with_head("h"));
        let mut walk = lock_ignoring_poison(&entry);
        walk.commits = (0..count).map(fake_commit).collect();
        drop(walk);
        cache.record_added(count);
    }

    #[test]
    fn eviction_keeps_the_entry_count_bounded_and_drops_the_oldest_shape() {
        let cache = LogPageCache::new();
        for index in 0..MAX_ENTRIES {
            fill(&cache, &format!("author-{index}"), 1);
        }
        assert_eq!(cache.len(), MAX_ENTRIES);

        // 再进一个新形状：最老的 author-0 应被驱逐
        fill(&cache, "author-new", 1);
        assert!(cache.len() <= MAX_ENTRIES, "条目数不得超过上限");

        // 被驱逐的形状重新请求时拿到的是**新的空前缀**（数据一致性由服务层的
        // 差分测试保证；这里只验证驱逐本身发生了）
        let revived = cache.entry(1, key_with_author("author-0"), fingerprint_with_head("h"));
        let walk = lock_ignoring_poison(&revived);
        assert!(
            walk.commits.is_empty(),
            "被驱逐后重新进入的条目应当是空前缀"
        );
    }

    #[test]
    fn touching_a_shape_keeps_it_alive_across_evictions() {
        let cache = LogPageCache::new();
        for index in 0..MAX_ENTRIES {
            fill(&cache, &format!("author-{index}"), 1);
        }
        // 把最早的两个形状标成最近使用，然后挤入一个新形状：
        // 被驱逐的应是"次新之外最久未用"的 author-2，而不是刚摸过的 author-0/1
        let _ = cache.entry(1, key_with_author("author-0"), fingerprint_with_head("h"));
        let _ = cache.entry(1, key_with_author("author-1"), fingerprint_with_head("h"));
        fill(&cache, "author-new", 1);
        assert!(cache.len() <= MAX_ENTRIES);

        let survivor = cache.entry(1, key_with_author("author-0"), fingerprint_with_head("h"));
        let walk = lock_ignoring_poison(&survivor);
        assert_eq!(walk.commits.len(), 1, "最近使用的条目不应被驱逐");
    }

    #[test]
    fn the_total_commit_budget_triggers_eviction_too() {
        // 单条目上限 60、总闸 100：两个 60 条的形状装不下，第二个进来的
        // 时候第一个必须被驱逐，账目才回得去预算内
        let cache = LogPageCache::new().with_entry_cap(60).with_total_cap(100);
        fill(&cache, "a", 60);
        fill(&cache, "b", 60);
        assert!(
            cache.len() < 2,
            "总闸超限时必须驱逐（当前条目数 {}）",
            cache.len()
        );
    }
}
