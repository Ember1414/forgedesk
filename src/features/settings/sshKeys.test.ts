import { describe, expect, it } from 'vitest';

import { agentKeys, agentTone, keyName, keyState, keyStateTone } from '@/features/settings/sshKeys';
import type { SshAgentStatus } from '@/lib/ipc';

/**
 * SSH 盘点的纯辅助函数（T2.7）。
 *
 * 这一层的判定会直接影响界面上的结论（"我的密钥配错了" / "agent 没开"），
 * 因此每条判断都单独钉住，而不是靠组件测试里的一句文本断言碰运气。
 */
describe('SSH 盘点的纯辅助函数', () => {
  it('从 Windows 绝对路径里取出文件名（两种分隔符都要认）', () => {
    // 后端返回的是本机绝对路径：只按 '/' 切会把整条路径当成文件名显示
    expect(
      keyName({
        publicPath: 'C:\\Users\\octocat\\.ssh\\id_ed25519.pub',
        privatePath: 'C:\\Users\\octocat\\.ssh\\id_ed25519',
      }),
    ).toBe('id_ed25519.pub');
  });

  it('从 POSIX 路径里取出文件名', () => {
    expect(keyName({ publicPath: '/home/octocat/.ssh/id_rsa.pub' })).toBe('id_rsa.pub');
  });

  it('公钥优先；两个路径都缺时给空串而不是抛错', () => {
    expect(keyName({ privatePath: '/home/u/.ssh/id_ed25519' })).toBe('id_ed25519');
    expect(keyName({})).toBe('');
  });

  it('判定配对状态', () => {
    expect(keyState({ publicPath: '/k.pub', privatePath: '/k' })).toBe('pair');
    // 只剩公钥是可能的（例如从别处拷了 .pub 回来），必须与"缺公钥"分开说
    expect(keyState({ publicPath: '/k.pub' })).toBe('publicOnly');
    expect(keyState({ privatePath: '/k' })).toBe('privateOnly');
  });

  it('只有"缺公钥"值得提醒', () => {
    // 配对齐全正常；只有公钥也不是故障（不影响使用，只是不能用来签名）
    expect(keyStateTone('pair')).toBe('ok');
    expect(keyStateTone('publicOnly')).toBe('info');
    expect(keyStateTone('privateOnly')).toBe('warning');
  });

  it('agent 的语气：没加载密钥是正常状态，agent 没运行才是警告', () => {
    const statuses: readonly (readonly [SshAgentStatus, 'ok' | 'info' | 'warning'])[] = [
      [{ kind: 'ready', keys: [] }, 'ok'],
      // 用户可能压根不打算用 agent：用警告色会把一件普通事说成故障
      [{ kind: 'noIdentities' }, 'info'],
      [{ kind: 'notRunning' }, 'warning'],
      // "问不出来"也算不确定，需要提示（reason 里带的是平台原因）
      [{ kind: 'unknown', reason: 'ssh-add not found' }, 'warning'],
    ];
    for (const [status, tone] of statuses) {
      expect(agentTone(status), JSON.stringify(status)).toBe(tone);
    }
  });

  it('非 ready 时没有已加载的密钥可展示', () => {
    expect(agentKeys({ kind: 'ready', keys: [{ fingerprint: 'SHA256:abc' }] })).toHaveLength(1);
    expect(agentKeys({ kind: 'notRunning' })).toHaveLength(0);
    expect(agentKeys({ kind: 'noIdentities' })).toHaveLength(0);
  });
});
