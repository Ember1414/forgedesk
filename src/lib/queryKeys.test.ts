import { describe, expect, it } from 'vitest';

import {
  DIFF_QUERY_KEY,
  LOG_QUERY_KEY,
  RECENT_REPOS_QUERY_KEY,
  SNAPSHOTS_QUERY_KEY,
  STATUS_QUERY_KEY,
  BRANCHES_QUERY_KEY,
  DIFF_KEY_PATH_INDEX,
  diffKeyPrefix,
  logKey,
  logKeyPrefix,
  statusKey,
} from '@/lib/queryKeys';

describe('logKey', () => {
  it('返回 [LOG_QUERY_KEY, repoId, cursor, filters] 四元组', () => {
    const key = logKey(42, 200, 'pageSize=200&paths=');
    expect(key).toEqual([LOG_QUERY_KEY, 42, 200, 'pageSize=200&paths=']);
    expect(key).toHaveLength(4);
  });

  it('首位恒为 LOG_QUERY_KEY', () => {
    expect(logKey(1, 0, '')[0]).toBe('log');
  });

  it('第二位为 repoId（数值）', () => {
    expect(logKey(7, 0, '')[1]).toBe(7);
  });

  it('第三位为 cursor（数值）', () => {
    expect(logKey(1, 500, 'sig')[2]).toBe(500);
  });

  it('第四位为 filters 签名字符串', () => {
    expect(logKey(1, 0, 'pageSize=200&revision="main"')[3]).toBe('pageSize=200&revision="main"');
  });
});

describe('logKeyPrefix', () => {
  it('返回 [LOG_QUERY_KEY, repoId] 二元组', () => {
    const prefix = logKeyPrefix(42);
    expect(prefix).toEqual([LOG_QUERY_KEY, 42]);
    expect(prefix).toHaveLength(2);
  });

  it('是 logKey 的前缀（结构化相等）', () => {
    const full = logKey(10, 0, 'sig');
    const prefix = logKeyPrefix(10);
    expect(full[0]).toBe(prefix[0]);
    expect(full[1]).toBe(prefix[1]);
  });
});

describe('statusKey', () => {
  it('返回 [STATUS_QUERY_KEY, repoId]', () => {
    expect(statusKey(5)).toEqual([STATUS_QUERY_KEY, 5]);
  });
});

describe('diffKeyPrefix', () => {
  it('返回 [DIFF_QUERY_KEY, repoId]', () => {
    expect(diffKeyPrefix(3)).toEqual([DIFF_QUERY_KEY, 3]);
  });
});

describe('常量', () => {
  it('DIFF_KEY_PATH_INDEX = 3', () => {
    expect(DIFF_KEY_PATH_INDEX).toBe(3);
  });

  it('各查询键互不相同', () => {
    const keys = new Set([
      STATUS_QUERY_KEY,
      DIFF_QUERY_KEY,
      LOG_QUERY_KEY,
      BRANCHES_QUERY_KEY,
      SNAPSHOTS_QUERY_KEY,
      RECENT_REPOS_QUERY_KEY,
    ]);
    expect(keys.size).toBe(6);
  });
});
