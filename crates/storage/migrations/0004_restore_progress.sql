-- 0004：回滚的"进行中"标记（M3 / T3.9）。
--
-- 回滚是**多阶段、会失败、可能被强杀**的过程：打保护点 → 复位 HEAD →
-- 复位索引 → 写回未跟踪内容 → 校验。如果应用在中间阶段被杀掉
-- （断电、OOM、用户强退），下一次启动时**必须知道**"上一次回滚没走完"——
-- 否则用户面对的是一个半新半旧的仓库，而且没有线索说明它为什么会这样。
--
-- 这三列就是那条线索：
--   restore_in_progress：布尔标记（0/1）。只在回滚期间为 1。
--   restore_stage：当时正在执行的阶段短名（见 RestoreStage::key：
--       protection / head / index / untracked / verify）。界面据此告诉用户
--       "上次停在'恢复索引'这一步"，而不是笼统一句"可能有问题"。
--   restore_started_at：Unix 毫秒。用来算"这次回滚卡了多久"——
--       一个卡了三天的标记显然不该被"继续"，而应被"放弃或人工检查"。
--
-- 为什么放在 snapshots 表而不是新建一张表：一个仓库同一时刻只可能有一次
-- 进行中的回滚（per-repo 互斥锁保证），而"是哪一次回滚"必须指向快照本身。
-- 独立表要额外维护外键与级联删除（快照被清理时标记也该一起消失），
-- 放在同一行天然成立：记录没了，标记也没了。
--
-- DEFAULT 0 让历史记录天然满足新约束（没有任何回滚正在进行）。
ALTER TABLE snapshots ADD COLUMN restore_in_progress INTEGER NOT NULL DEFAULT 0;
ALTER TABLE snapshots ADD COLUMN restore_stage TEXT;
ALTER TABLE snapshots ADD COLUMN restore_started_at INTEGER;
