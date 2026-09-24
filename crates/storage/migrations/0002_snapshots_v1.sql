-- 0002：快照 v1（M1 / T1.9）。
-- 0001 已建 snapshots 表；这里补上 v1 记录需要的列：
--   branch / detached / operation_state：回滚时要知道"当时在哪个分支、是否游离、
--     仓库是否正处于合并/变基中途"——只恢复 oid 而丢掉分支语境，会让用户落在一个
--     意料之外的 HEAD 上；
--   untracked_paths：未跟踪文件清单（JSON 数组）。v1 只记路径不备份内容
--     （M1 的破坏性操作路径不会动未跟踪文件；内容备份属于 T3.8 快照 v2）。
-- 为什么用 ALTER TABLE 而不是重建表：已有用户库里可能有真实快照，重建意味着
-- 迁移代码要负责搬数据——多一份出错的可能，换不来任何好处。
ALTER TABLE snapshots ADD COLUMN branch TEXT;
ALTER TABLE snapshots ADD COLUMN detached INTEGER NOT NULL DEFAULT 0;
ALTER TABLE snapshots ADD COLUMN operation_state TEXT;
ALTER TABLE snapshots ADD COLUMN untracked_paths TEXT;
