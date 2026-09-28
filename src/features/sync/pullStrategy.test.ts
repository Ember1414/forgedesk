import { describe, expect, it } from 'vitest';

import {
  DEFAULT_PULL_STRATEGY,
  PULL_STRATEGIES,
  PULL_STRATEGY_SETTING_KEY,
  isPullStrategy,
  parsePullStrategy,
  serializePullStrategy,
} from '@/features/sync/pullStrategy';

/**
 * 拉取策略的读写必须**只会**产生三种合法取值之一。
 *
 * 为什么值得单测：这个值来自本地设置（用户能直接改数据库，别的版本也可能写过），
 * 一旦被原样带进 `git_pull`，后端会拒绝整次拉取——用户看到的是"拉取失败"，
 * 而真正的原因是半个 JSON 串。
 */
describe('parsePullStrategy', () => {
  it('三种合法策略都能往返', () => {
    for (const strategy of PULL_STRATEGIES) {
      expect(parsePullStrategy(serializePullStrategy(strategy))).toBe(strategy);
    }
  });

  it('没存过时使用缺省策略（仅快进）', () => {
    expect(DEFAULT_PULL_STRATEGY).toBe('fastForwardOnly');
    expect(parsePullStrategy(null)).toBe('fastForwardOnly');
    expect(parsePullStrategy(undefined)).toBe('fastForwardOnly');
    expect(parsePullStrategy('')).toBe('fastForwardOnly');
  });

  it('坏值与未知取值回退到缺省，而不是抛错', () => {
    // 不是合法 JSON（被手工改成裸字符串）
    expect(parsePullStrategy('merge')).toBe('fastForwardOnly');
    // 是合法 JSON 但不是已知取值
    expect(parsePullStrategy('"fast-forward"')).toBe('fastForwardOnly');
    expect(parsePullStrategy('{"strategy":"merge"}')).toBe('fastForwardOnly');
    expect(parsePullStrategy('null')).toBe('fastForwardOnly');
  });
});

describe('isPullStrategy', () => {
  it('只认三个稳定取值（大小写与类型都严格）', () => {
    expect(isPullStrategy('fastForwardOnly')).toBe(true);
    expect(isPullStrategy('merge')).toBe(true);
    expect(isPullStrategy('rebase')).toBe(true);
    expect(isPullStrategy('FastForwardOnly')).toBe(false);
    expect(isPullStrategy('fast-forward-only')).toBe(false);
    expect(isPullStrategy(3)).toBe(false);
    expect(isPullStrategy(null)).toBe(false);
  });
});

describe('设置键', () => {
  it('落在 sync 命名域里（设置页可按前缀归类）', () => {
    expect(PULL_STRATEGY_SETTING_KEY).toBe('sync.pullStrategy');
  });
});
