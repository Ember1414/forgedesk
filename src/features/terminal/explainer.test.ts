import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import {
  EXPLAIN_CATEGORIES,
  EXPLAIN_COMMANDS,
  EXPLAIN_ENTRY_COUNT,
  explainGitCommand,
  tokenizeCommand,
} from '@/features/terminal/explainer';

describe('tokenizeCommand（分词）', () => {
  it('按空白分词，引号内空白保留', () => {
    expect(tokenizeCommand('git commit -m "two words"')).toEqual([
      'git',
      'commit',
      '-m',
      'two words',
    ]);
  });

  it('空行与纯空白返回空数组', () => {
    expect(tokenizeCommand('   ')).toEqual([]);
  });
});

describe('explainGitCommand（命令解释匹配）', () => {
  it('解释基础命令（≥30 条匹配用例中的命令级）', () => {
    for (const line of [
      'git init',
      'git clone https://example.com/r.git',
      'git status',
      'git add src/main.rs',
      'git restore --staged a.rs',
      'git commit -m "msg"',
      'git branch feature/x',
      'git checkout main',
      'git switch main',
      'git merge feature/x',
      'git rebase main',
      'git cherry-pick abc1234',
      'git revert abc1234',
      'git reset --soft HEAD~1',
      'git reflog',
      'git stash push -m wip',
      'git fetch origin',
      'git pull origin main',
      'git push origin main',
      'git remote -v',
      'git log --oneline',
      'git diff --staged',
      'git show abc1234',
      'git blame src/main.rs',
      'git bisect start',
      'git tag v1.2.0',
      'git config user.name x',
      'git gc',
      'git fsck',
      'git worktree add ../wt main',
      'git submodule update --init',
      'git lfs track "*.psd"',
    ]) {
      expect(explainGitCommand(line), line).not.toBeNull();
    }
  });

  it('解释子命令与关键参数（≥30 条匹配用例中的子级）', () => {
    const cases: readonly [string, string][] = [
      ['git reset --hard', '--hard'],
      ['git reset --soft HEAD~1', '--soft'],
      ['git reset --mixed HEAD~1', '--mixed'],
      ['git branch -D feature/x', '-D'],
      ['git branch -d feature/x', '-d'],
      ['git checkout -f main', '-f'],
      ['git checkout -b feature/new', '-b'],
      ['git switch -c feature/new', '-c'],
      ['git switch --detach v1.0', '--detach'],
      ['git merge --abort', '--abort'],
      ['git rebase --abort', '--abort'],
      ['git rebase --continue', '--continue'],
      ['git rebase -i main', '-i'],
      ['git cherry-pick --abort', '--abort'],
      ['git stash drop stash@{0}', 'drop'],
      ['git stash clear', 'clear'],
      ['git stash pop', 'pop'],
      ['git stash apply stash@{1}', 'apply'],
      ['git push --force origin main', '--force'],
      ['git push --force-with-lease origin main', '--force-with-lease'],
      ['git push origin --delete feature/x', '--delete'],
      ['git remote add upstream https://x.com/r.git', 'add'],
      ['git remote remove upstream', 'remove'],
      ['git log --graph --oneline', '--graph'],
      ['git diff --staged', '--staged'],
      ['git bisect reset', 'reset'],
      ['git tag -d v1.0', '-d'],
      ['git config --global user.email x', '--global'],
      ['git gc --prune=now', '--prune=now'],
      ['git update-ref -d refs/heads/x', '-d'],
      ['git restore --source HEAD~1 a.rs', '--source'],
      ['git restore --staged a.rs', '--staged'],
      ['git add -A', '-A'],
      ['git rm --cached build.log', '--cached'],
      ['git commit --amend', '--amend'],
      ['git commit --no-verify -m m', '--no-verify'],
      ['git clean -n', '-n'],
      ['git worktree remove ../wt', 'remove'],
      ['git clone --depth 1 https://x.com/r.git', '--depth'],
      ['git fetch --prune', '--prune'],
      ['git reflog expire --expire=now --all', 'expire'],
    ];
    for (const [line, expected] of cases) {
      const result = explainGitCommand(line);
      expect(result?.sub?.name, line).toBe(expected);
    }
  });

  it('带路径与全局选项的 git 也能匹配', () => {
    expect(explainGitCommand('git -C ../other reset --hard')?.sub?.name).toBe('--hard');
    expect(
      explainGitCommand('"C:\\Program Files\\Git\\bin\\git.exe" push --force')?.sub?.name,
    ).toBe('--force');
    expect(explainGitCommand('git -c core.autocrlf=false status')?.command.risk).toBe('safe');
  });

  it('非 git 输入与未知命令返回 null', () => {
    for (const line of [
      'ls -la',
      'npm run build',
      'echo git status',
      'git',
      '',
      'git not-a-known-command',
    ]) {
      expect(explainGitCommand(line), line).toBeNull();
    }
  });

  it('已知命令的未知参数仍解释命令本身（子级为 null）', () => {
    const result = explainGitCommand('git status --not-a-known-flag');
    expect(result?.command.summary.length).toBeGreaterThan(0);
    expect(result?.sub).toBeNull();
  });

  it('危险条目与等价图形入口的映射正确', () => {
    const hard = explainGitCommand('git reset --hard');
    expect(hard?.sub?.risk).toBe('dangerous');
    expect(hard?.sub?.equivalentUiRoute).toBe('/snapshots');

    const log = explainGitCommand('git log');
    expect(log?.command.equivalentUiRoute).toBe('/history');
  });
});

describe('知识库本身（数据完整性）', () => {
  it('条目总数 ≥ 60（任务书验收线）', () => {
    expect(EXPLAIN_ENTRY_COUNT).toBeGreaterThanOrEqual(60);
    expect(EXPLAIN_COMMANDS.length).toBeGreaterThanOrEqual(30);
  });

  it('每个条目都有分类、风险与解释；分类集合稳定', () => {
    for (const entry of EXPLAIN_COMMANDS) {
      expect(entry.summary.length).toBeGreaterThan(0);
      expect(['safe', 'caution', 'dangerous']).toContain(entry.risk);
    }
    expect(EXPLAIN_CATEGORIES).toEqual([
      'repo',
      'staging',
      'commit',
      'branch',
      'merge',
      'history',
      'stash',
      'remote',
      'config',
      'maintenance',
    ]);
  });

  it('docs_url 全部为 https 官方文档域名', () => {
    const allDocs = EXPLAIN_COMMANDS.flatMap((entry) => [entry.docsUrl]);
    for (const url of allDocs) {
      if (url !== undefined) {
        expect(url.startsWith('https://git-scm.com/docs')).toBe(true);
      }
    }
  });
});

describe('红线 R1 / 零网络（grep 断言）', () => {
  /** 任何网络调用的痕迹：匹配即失败（依赖面由 compliance 门禁另行盯住）。 */
  const NETWORK_PATTERNS: readonly [string, RegExp][] = [
    ['fetch(', /\bfetch\s*\(/],
    ['XMLHttpRequest', /XMLHttpRequest/],
    ['WebSocket', /WebSocket/],
    ['axios', /\baxios\b/],
    ['http.request', /http[s]?\.request|\.get\(['"]http|\.post\(['"]http/],
    ['EventSource', /EventSource/],
    ['sendBeacon', /sendBeacon/],
  ];

  it('explainer.ts 与知识库 YAML 中不存在任何网络调用', () => {
    const sources = [
      readFileSync(join(process.cwd(), 'src/features/terminal/explainer.ts'), 'utf-8'),
      readFileSync(join(process.cwd(), 'assets/git-explains.yaml'), 'utf-8'),
    ];
    for (const [name, pattern] of NETWORK_PATTERNS) {
      for (const source of sources) {
        expect(pattern.test(source), `${name} 不允许出现在本地知识库链路里`).toBe(false);
      }
    }
  });

  it('yaml 依赖只被用于解析本地字符串（无运行时取回）', () => {
    const source = readFileSync(join(process.cwd(), 'src/features/terminal/explainer.ts'), 'utf-8');
    // 数据来自构建期 ?raw 打包，而不是任何 URL
    expect(source).toContain("?raw'");
    expect(/\bfetch\s*\(/.test(source)).toBe(false);
  });
});
