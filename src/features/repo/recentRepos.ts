/**
 * 最近打开的仓库（M0 占位数据）。
 *
 * 为什么现在就有这个模块：外壳的仓库切换器、状态栏、导航的可用性都依赖
 * "当前仓库"，而真实的仓库列表要等 T0.7（SQLite + 迁移框架）落地。
 * 与其在各处塞 `'demo'` 这类魔法字符串，不如把占位数据集中一处，
 * 后续把数据源换成 TanStack Query 查询即可，调用方无需改动。
 *
 * 注意：这是**示例数据**，界面上会明确标注（见 shell.titleBar.repoSwitcher.placeholderNote），
 * 不能让用户误以为真的打开过这些仓库。
 */
export interface RecentRepo {
  readonly id: string;
  readonly name: string;
  /** 仓库路径（Windows 下为盘符路径，其他平台为 POSIX 路径）。 */
  readonly path: string;
  /** 上次已知的当前分支；真实值由 git 读取（T1.x）。 */
  readonly defaultBranch: string;
}

export const PLACEHOLDER_RECENT_REPOS: readonly RecentRepo[] = [
  {
    id: 'example-forgedesk',
    name: 'forgedesk',
    path: 'E:\\Projects\\ForgeDesk',
    defaultBranch: 'main',
  },
  {
    id: 'example-notes',
    name: 'notes',
    path: '~/Documents/notes',
    defaultBranch: 'trunk',
  },
];

/** 按 id 查占位仓库；未找到返回 undefined。 */
export function findRecentRepo(repoId: string | null): RecentRepo | undefined {
  if (repoId === null) {
    return undefined;
  }
  return PLACEHOLDER_RECENT_REPOS.find((repo) => repo.id === repoId);
}
