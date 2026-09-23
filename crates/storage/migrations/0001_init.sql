-- ============================================================================
-- 0001_init：M0/T0.7 建立的初始结构（对应 docs/PLAN.md §5.10）
--
-- 约定：
--   1. 表名与字段名沿用 PLAN §5.10，便于对照；此处只建表，不含业务逻辑。
--   2. 时间统一存**Unix 毫秒**（INTEGER）。为什么不用文本时间：
--      排序、区间查询、跨时区比较都更简单，展示层再格式化。
--   3. 所有 CREATE 都带 IF NOT EXISTS：迁移可能被中断后重跑，
--      幂等比"假设一定干净"更可靠（迁移器另有版本记录，这里是第二道保险）。
--   4. 枚举型字段（kind / result / size_class 等）用 TEXT 存稳定字符串，
--      不建 CHECK 约束：取值集合会随里程碑增长，约束反而会拦住合法的演进。
-- ============================================================================

CREATE TABLE IF NOT EXISTS schema_migrations (
  version    INTEGER PRIMARY KEY,
  name       TEXT    NOT NULL,
  applied_at INTEGER NOT NULL
);

-- 仓库：本应用管理过的本地仓库。path 唯一，避免同一个仓库被登记两次。
CREATE TABLE IF NOT EXISTS repositories (
  id             INTEGER PRIMARY KEY,
  path           TEXT    NOT NULL UNIQUE,
  name           TEXT    NOT NULL,
  default_branch TEXT,
  provider_id    TEXT,
  last_opened_at INTEGER,
  size_class     TEXT,
  created_at     INTEGER NOT NULL
);

-- 设置：scope 区分全局与仓库级，值为 JSON 字符串（由上层决定 schema）。
CREATE TABLE IF NOT EXISTS settings (
  scope   TEXT    NOT NULL,
  repo_id INTEGER,
  key     TEXT    NOT NULL,
  value   TEXT    NOT NULL,
  PRIMARY KEY (scope, repo_id, key)
);

-- ⚠️ SQLite 的坑：在 UNIQUE / PRIMARY KEY 中，NULL 之间**互不相等**，
-- 因此 `PRIMARY KEY (scope, repo_id, key)` 对 repo_id IS NULL（全局设置）
-- 完全不起作用——同一全局键可以插入任意多行，`INSERT OR REPLACE` 也不会覆盖，
-- 表现为"设置改了但读出来的还是旧值"这种极难定位的问题。
-- 下面这个索引把 NULL 折叠成一个具体值，让唯一性对全局设置同样成立。
CREATE UNIQUE INDEX IF NOT EXISTS idx_settings_unique
  ON settings(scope, COALESCE(repo_id, -1), key);

CREATE INDEX IF NOT EXISTS idx_settings_scope ON settings(scope, repo_id);

-- 操作快照：破坏性操作前的状态锚点（回滚依据，见 PLAN §5.10 的快照策略）。
CREATE TABLE IF NOT EXISTS snapshots (
  id             INTEGER PRIMARY KEY,
  repo_id        INTEGER NOT NULL,
  label          TEXT    NOT NULL,
  kind           TEXT    NOT NULL,
  head_oid       TEXT    NOT NULL,
  index_tree_oid TEXT,
  reflog_ref     TEXT,
  backup_path    TEXT,
  checksum       TEXT,
  created_at     INTEGER NOT NULL
);

-- 操作记录：每一次写操作的审计与回放依据（M1 起写入）。
CREATE TABLE IF NOT EXISTS operation_records (
  id             INTEGER PRIMARY KEY,
  repo_id        INTEGER NOT NULL,
  op_type        TEXT    NOT NULL,
  args_json      TEXT,
  started_at     INTEGER,
  ended_at       INTEGER,
  exit_code      INTEGER,
  stderr_summary TEXT,
  snapshot_id    INTEGER,
  reversible     INTEGER DEFAULT 1
);

-- 托管平台账号。凭据本体在系统 keyring，这里只存引用（红线 R8）。
CREATE TABLE IF NOT EXISTS accounts (
  id             TEXT PRIMARY KEY,
  provider       TEXT NOT NULL,
  host           TEXT NOT NULL,
  login          TEXT NOT NULL,
  avatar_url     TEXT,
  scopes         TEXT,
  credential_ref TEXT NOT NULL,
  created_at     INTEGER
);

-- 平台 API 响应的 ETag 缓存（降低限流风险）。
CREATE TABLE IF NOT EXISTS api_cache (
  key        TEXT PRIMARY KEY,
  provider   TEXT,
  etag       TEXT,
  payload    BLOB,
  expires_at INTEGER,
  updated_at INTEGER
);

-- 审计日志：谁在什么时候对哪个仓库做了什么（REDLINE R7 的可追溯性）。
CREATE TABLE IF NOT EXISTS audit_log (
  id          INTEGER PRIMARY KEY,
  ts          INTEGER NOT NULL,
  repo_id     INTEGER,
  actor       TEXT,
  action      TEXT NOT NULL,
  detail_json TEXT,
  result      TEXT
);

CREATE INDEX IF NOT EXISTS idx_op_repo_ts   ON operation_records(repo_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_snap_repo_ts ON snapshots(repo_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_ts     ON audit_log(ts DESC);
