import { describe, expect, it } from 'vitest';

import { LineTracker, isGitCommand } from '@/features/terminal/gitInputRefresh';

describe('LineTracker（键入行跟踪）', () => {
  it('整行输入 + Enter 提交一行', () => {
    const tracker = new LineTracker();
    expect(tracker.feed('git status\r')).toEqual(['git status']);
    expect(tracker.feed('git log -5\r')).toEqual(['git log -5']);
  });

  it('一行被拆成多段输入也能正确拼回', () => {
    const tracker = new LineTracker();
    expect(tracker.feed('git com')).toEqual([]);
    expect(tracker.feed('mit -m "msg"\r')).toEqual(['git commit -m "msg"']);
  });

  it('退格删除已输入的字符', () => {
    const tracker = new LineTracker();
    expect(tracker.feed('git statuz\x7f\r')).toEqual(['git statu']);
  });

  it('Ctrl+C 与 Ctrl+U 清空当前行，不产生提交', () => {
    const tracker = new LineTracker();
    expect(tracker.feed('git reset --hard\x15git log\r')).toEqual(['git log']);
    expect(tracker.feed('git push\x03more\r')).toEqual(['more']);
  });

  it('方向键的 ESC 字节被忽略，后续可打印字节按原样进入缓冲（已知失真，见文件头）', () => {
    const tracker = new LineTracker();
    // ESC 本身被丢弃，但 [ D [ C 是可打印字符——方向键补全历史会让行缓冲
    // 失真，这是刻意的取舍：判定宁可漏报不可误伤，刷新兜底靠文件监听
    expect(tracker.feed('git st\x1b[D\x1b[Catus\r')).toEqual(['git st[D[Catus']);
  });
});

describe('isGitCommand（git 命令判定）', () => {
  it('git 及常见子命令命中', () => {
    for (const line of [
      'git status',
      ' git push origin main',
      'git',
      'git.exe status',
      'GIT STATUS',
    ]) {
      expect(isGitCommand(line), line).toBe(true);
    }
  });

  it('含 git 字样的其他命令不命中（宁可漏报不可误伤）', () => {
    for (const line of [
      'legit status',
      'echo git',
      'npm run git:check',
      'gitignore-gen',
      'ls',
      '',
    ]) {
      expect(isGitCommand(line), line).toBe(false);
    }
  });
});
