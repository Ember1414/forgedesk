//! 提交历史（M2）。
//!
//! 这里放的是**纯逻辑**：历史查询条件的形状、以及把一串提交摆成图（泳道布局）。
//! 真正的 `git log` 调用在 `git-engine`，编排在 `services`——
//! 分层与 `git` 模块一致（domain 不认识 libgit2，也不认识进程）。

mod layout;

pub use layout::{layout, EdgeKind, GraphEdge, GraphLayout, GraphRow, LayoutMode, PALETTE_SIZE};
