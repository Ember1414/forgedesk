/**
 * SSH 盘点结果的**纯**辅助函数（T2.7）。
 *
 * 抽出来的理由与 `syncStatus.ts` 相同：判定逻辑（配对状态、agent 该用什么语气提示）
 * 与渲染无关，单独测比在组件里用 `expect(container).toHaveTextContent` 试出来更可靠，
 * 也更能覆盖边界（Windows 路径、只有公钥、agent 问不到）。
 */
import type { SshAgentKey, SshAgentStatus, SshKeyInfo } from '@/lib/ipc';

/** 一把密钥的配对状态。 */
export type SshKeyState = 'pair' | 'publicOnly' | 'privateOnly';

/** 密钥的文件名（公钥优先；两种路径分隔符都要认）。 */
export function keyName(key: SshKeyInfo): string {
  const path = key.publicPath ?? key.privatePath ?? '';
  // 后端给的是**本地绝对路径**：Windows 上是 `C:\Users\…\.ssh\id_ed25519`，
  // 只按 `/` 切会把整条路径当成文件名显示出来（既有隐私观感问题，也难读）
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? '';
}

/** 判定配对状态（`privatePath` / `publicPath` 的存在性由后端给出）。 */
export function keyState(key: SshKeyInfo): SshKeyState {
  if (key.publicPath !== undefined && key.privatePath !== undefined) {
    return 'pair';
  }
  return key.publicPath !== undefined ? 'publicOnly' : 'privateOnly';
}

/** 配对状态的提示语气：只有"缺公钥"值得提醒（可能是复制密钥时漏了 `.pub`）。 */
export function keyStateTone(state: SshKeyState): 'ok' | 'info' | 'warning' {
  if (state === 'pair') {
    return 'ok';
  }
  return state === 'privateOnly' ? 'warning' : 'info';
}

/** agent 状态对应的提示语气。 */
export function agentTone(agent: SshAgentStatus): 'ok' | 'info' | 'warning' {
  switch (agent.kind) {
    case 'ready':
      return 'ok';
    // "agent 在跑但没加载密钥"是**正常状态**（用户可能根本不打算用它），
    // 用警告色会把一件普通的事说成故障
    case 'noIdentities':
      return 'info';
    case 'notRunning':
    case 'unknown':
      return 'warning';
    default: {
      // 穷举兜底：将来后端新增 kind 时编译期就会在这里报错（`never` 不可赋值）
      const unreachable: never = agent;
      return unreachable;
    }
  }
}

/** agent 里已加载的密钥（非 ready 时为空数组，便于渲染时统一处理）。 */
export function agentKeys(agent: SshAgentStatus): readonly SshAgentKey[] {
  return agent.kind === 'ready' ? agent.keys : [];
}
