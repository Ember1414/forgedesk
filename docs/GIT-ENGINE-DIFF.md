# Git 引擎双实现差异报告

> 本文件由 **M1 / T1.2** 产出，记录 `CliGitEngine`（系统 git CLI）与 `Libgit2Engine`
> （libgit2）在**同一份仓库**上的语义差异。**M1 / T1.3** 追加了 `discover` 相关的
> 能力差异（见 §4）。
>
> 它存在的理由：两个引擎同时存在于产品里（读走 libgit2、写走 CLI），
> 因此"同一份状态经两条路径必须得到同一个结论"。差异不会报错，只会让界面
> 时而显示 A、时而显示 B——那是用户无法自助排查的一类问题。
>
> **T2.10 后的特例**：状态读（`WorkspaceService::status`）按索引条目数分流——
> 大仓库（≥ [`STATUS_CLI_ENTRY_THRESHOLD`] = 2000）切到 CLI（libgit2 在该形状上
> 慢约 10 倍，见 `docs/PERF-BASELINE.md` §3.1），其余仍走 libgit2。两条路径的
> 结果一致性由差分测试逐仓库对拍；富化对两条路径共用，界面无字段差异。
>
> 配套测试：`crates/git-engine/tests/differential.rs`（16 项通过、1 项 `#[ignore]`）
> 与 `crates/git-engine/tests/discover.rs`（15 项通过）。

---

## 1. 对比口径（"一致"的定义）

两个引擎的信息量**本来就不一样**，因此不是逐字段相等，而是"在两者都能给出的
语义上相等"。规范化规则如下（同时写在测试文件头部）：

| 对象 | 比较内容 | 规范化处理 |
| --- | --- | --- |
| 状态 | `(路径, 标记, 来源路径)` 的**集合** | 未跟踪统一记为 `??`（porcelain 用 `?`，libgit2 用 `WT_NEW` 位）；冲突统一记为 `conflicted`（porcelain 能区分 `UU`/`AA`/`DU`，libgit2 只有 `CONFLICTED` 位） |
| diff | `(路径, 来源路径, 新增行, 删除行, 是否二进制)` 的**集合** | 集合比较：两个引擎的条目顺序不保证一致 |
| log | oid **序列**，以及每个 oid 的父提交、subject、提交时间 | 序列比较（顺序是语义的一部分）；`signature` 不参与比较。`refs` 自 T2.10 起在 `show` 侧比较（排序后的 token 序列） |

### 测试夹具的确定性措施

- 所有提交通过 `GIT_AUTHOR_DATE` / `GIT_COMMITTER_DATE` 固定且**逐个递增**：
  同一秒内的多个提交在 git 与 libgit2 里的排序平局规则可能不同，
  那会让"提交顺序"产生假阳性。
- `core.autocrlf=false`：行尾转换会让同一工作区产生不同的 blob。
- 身份固定为 `Fixture Author <author@example.com>`。
- 六类仓库：① 线性 ② 多分叉 + 合并 ③ 重命名 + 删除 ④ 二进制 ⑤ 子模块 ⑥ 1000+ 文件，
  外加一个空仓库。

---

## 2. 结论

**六类仓库 + 空仓库的 status / diff / log 全部一致**，只有一处例外（见 §3）。

过程中发现并修复了三处**实现缺陷**（不是引擎差异，是代码写错了）：

| # | 症状 | 根因 | 修复 |
| --- | --- | --- | --- |
| 1 | libgit2 的 diff 把每个文件都显示成重命名 | `delta.old_file().path()` 在普通修改上也有值 | 只在 `Renamed`/`Copied` 时填 `original_path` |
| 2 | libgit2 的状态把重命名显示成"旧文件重命名成自己" | `StatusEntry::path()` 给的是**来源**路径，与 porcelain 的约定相反 | 重命名条目改取 `delta.new_file()` |
| 3 | 空仓库上两个引擎的 `log` 都报错 | `git log` 与 `push_ref` 在 HEAD 尚未诞生时都会失败 | 两边统一：默认 HEAD 解析不出来 → 空页；显式指定修订名失败 → 仍然报错 |

第 3 条特意保留了这个区分：把"修订名拼错"也变成空页会掩盖调用方的错误。

---

## 3. 已知差异（1 处，已用 `#[ignore]` 钉住）

### 3.1 子模块（gitlink）的增删行数

| 引擎 | `FileDiff` |
| --- | --- |
| CLI | `path="vendor/sub", additions=0, deletions=0, binary=false` |
| libgit2 | `path="vendor/sub", additions=1, deletions=1, binary=false` |

**原因**：gitlink 不是文件内容，`git diff --numstat` 对它的处理是"不统计行数"
（输出 `0\t0`），而 libgit2 的 `Patch::line_stats` 把它当成一次普通的
"一行替换"。两者都不是错误，但**数字确实不同**。

**处置决策**：界面**不得**依赖子模块条目的增删行数。
- 状态面板对子模块应显示"提交已变更 / 有本地修改 / 有未跟踪文件"这三个标记
  （`SubmoduleState`），而不是行数；
- 测试侧：`submodules_are_consistent_across_engines` 只比较 status 与 log，
  diff 的比较放在 `submodule_diff_line_counts_differ_between_engines`，
  该测试标记 `#[ignore]` 并在失败信息里指向本节——差异是**被记录**的，
  而不是被规范化规则悄悄抹掉的。

---

## 4. 信息量差异（不是"不一致"，是能力边界）

下列字段两个引擎**不是都能给出**，因此不参与对比。界面必须以
**libgit2 的能力为准**（读路径由它承担）：

| 字段 | CLI | libgit2 | 原因 |
| --- | --- | --- | --- |
| `FileChange` 的模式与 oid（`mode_*` / `oid_*`） | 有值 | 全为 `None` | libgit2 的状态 API 不暴露它们 |
| 冲突条目的 `XY` | `UU` / `AA` / `DU` / `UD` / `AU` / `UA` / `DD` | 统一 `UU` | libgit2 只有 `CONFLICTED` 位，不区分冲突类型 |
| ~~`Commit.refs`~~（T2.10 起移出本表） | 有值（`%D`） | 有值（`RefDecorations`，token 形状与排序模仿 `%D`） | 曾经 libgit2 侧留空（历史图 ref 胶囊因此全空，T2.10 修复的 P1）；现在一次引用枚举建装饰表（O(引用数)，<1ms），差分测试按"排序后 token 序列相等"钉住 |
| `Commit.signature` | `%G?` 的真实结果 | `Unknown` | libgit2 不做 GPG 校验 |
| `Branch.upstream_gone` | 可判定（`[gone]`） | 恒为 `false` | libgit2 无法区分"没有上游"与"上游已删除" |
| `Tag.message` | 附注标签有值 | 附注标签有值 | 一致（轻量标签两边都不填：`%(contents:subject)` 给的是提交标题，不是标签信息） |
| `RepositoryInfo.worktrees`（T1.3） | 完整列表（主 + 关联工作区，含路径 / HEAD / 分支 / locked / prunable） | **只有主工作区** | `git2::Repository::worktrees` 返回的是 `StringArray`（只有工作区**名称**），既没有路径也没有 HEAD，而 `git2` 没有暴露 `git_worktree_lookup` |
| `remote_refs_containing`（T1.8） | 已实现（`for-each-ref --contains HEAD refs/remotes`） | 返回 `UNSUPPORTED_BY_ENGINE` | 与 `commit` 同族：它服务的是"这次改写会不会影响远端"这个**写路径**判断，而 libgit2 侧要自己遍历 refs 做可达性计算（还要单独处理"相等"这一 libgit2 API 不覆盖的边界），收益不抵两套实现之间产生分歧的风险 |
| `LogQuery.follow_renames`（T2.1，`--follow`） | 支持（`paths` 恰好一条时传 `--follow`） | 返回 `UNSUPPORTED_BY_ENGINE` | libgit2 没有 `--follow` 等价物；装作支持等于悄悄给出**错误结果**（漏掉重命名前的历史），宁可明确拒绝。由 `log_follow_renames_is_unsupported_by_libgit2` 钉住 |

**给 `services` 层的约束**：需要上述字段的功能，必须走 CLI 引擎，
或者由 CLI 引擎补一次查询；不得假设"换个引擎也有这些值"。

### 4.1 `discover` 为什么走 CLI

`RepositoryInfo.worktrees` 的能力缺口（上表最后一行）意味着**用 libgit2 做
`discover` 会让关联工作区永远不显示**。因此 `services::repository` 的
`discover` / `open` 走 **CLI 引擎**，理由有三条：

1. 打开仓库是**一次性的、用户发起**的探测，不是高频调用——libgit2"省一次进程"
   的优势在这里几乎不存在（`status` / `diff` / `log` 仍然走 libgit2）；
2. 同一次"打开"还要做**仓库配置审计**（`git config --local --list`）与
   **git 版本检查**（`git --version`），两者都只有 CLI 侧能做，
   合并成一次 CLI 探测比"libgit2 查一半 + CLI 查一半"更少往返；
3. `git worktree list --porcelain` 与 `git rev-parse --is-shallow-repository`
   是 git 的稳定机器可读接口，解析器已有穷举测试（`parsers::worktree`）。

`Libgit2Engine::discover` 仍然实现（trait 要求），并填充它能填的字段
（`is_shallow` / `is_lfs` / `default_branch` / 主工作区），差异由
`tests/discover.rs::libgit2_reports_only_the_main_worktree` 钉住。

---

## 5. 后续任务

- **T1.5（diff 解析）**：行级内容（`DiffHunk` / `DiffLine`）在两个实现里都还是空的。
  届时需要把 libgit2 的 `Patch::from_diff` 行回调与 CLI 的 unified diff 解析器
  也纳入差分对比——那是本次没有覆盖的最大一块。
- **M2（DAG）**：`log` 的 `--all` 与过滤器一致性已在 M2 补测（2026-09 批次：
  `log_all_branches_is_consistent_across_engines`、
  `log_since_and_until_bounds_are_inclusive_and_consistent_across_engines`、
  `log_message_contains_is_consistent_across_engines`、
  `log_first_parent_only_is_consistent_across_engines` 四项对拍，外加
  `log_follow_renames_is_unsupported_by_libgit2` 钉住能力边界，见 §4）；
  仍待补测的是拓扑排序一致性（多 ref 下的排序平局规则更容易分叉）。
- **M5（诊断）**：`map_error` 目前复用 `ErrorCode::classify`，两个引擎对同一类
  失败给出同一个错误码。新增诊断规则时要在两边的错误路径上都验证一遍。
