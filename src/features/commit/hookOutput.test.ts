import { describe, expect, it } from 'vitest';

import { analyzeHookOutput, classifyLine } from '@/features/commit/hookOutput';

describe('钩子输出结构化', () => {
  it('识别 ESLint 的风格化错误行与摘要', () => {
    const report = analyzeHookOutput(
      [
        '/repo/src/a.ts',
        '  3:5  error  Unexpected console statement  no-console',
        '  7:1  warning  Missing return type  @typescript-eslint/explicit-module-boundary-types',
        '',
        '✖ 2 problems (1 error, 1 warning)',
      ].join('\n'),
    );

    expect(report.phase).toBe('code-check');
    expect(report.tools).toContain('eslint');
    expect(report.errorCount).toBe(2);
    expect(report.warningCount).toBe(1);
    expect(report.lines[1]?.kind).toBe('error');
    expect(report.lines[2]?.kind).toBe('warning');
  });

  it('识别 TypeScript 的编译错误', () => {
    const report = analyzeHookOutput(
      ['src/a.ts(3,5): error TS2322: Type string is not assignable to type number.'].join('\n'),
    );

    expect(report.phase).toBe('code-check');
    expect(report.tools).toContain('typescript');
    expect(report.errorCount).toBe(1);
  });

  it('提交信息检查的输出归到 message-check', () => {
    const report = analyzeHookOutput(
      [
        'commit-msg: the subject line must not be empty',
        'hint: fix the message and try again',
      ].join('\n'),
    );

    expect(report.phase).toBe('message-check');
    // git 自己的 hint 不是失败原因
    expect(report.lines[1]?.kind).toBe('info');
  });

  it('依赖安装失败归到 dependencies', () => {
    const report = analyzeHookOutput(
      ['npm ERR! code ERESOLVE', 'npm ERR! Cannot find module "left-pad"'].join('\n'),
    );

    expect(report.phase).toBe('dependencies');
  });

  it('认不出的输出不瞎猜阶段', () => {
    const report = analyzeHookOutput('something went wrong\nplease check the docs');

    expect(report.phase).toBe('unknown');
    expect(report.tools).toEqual([]);
    expect(report.errorCount).toBe(0);
  });

  it('区分 error 与 warning 两类行', () => {
    expect(classifyLine('  3:5  error  nope')).toBe('error');
    expect(classifyLine('  3:5  warning  meh')).toBe('warning');
    expect(classifyLine('hint: try again')).toBe('info');
    expect(classifyLine('plain output')).toBe('plain');
    expect(classifyLine('   ')).toBe('plain');
  });

  it('空输出不产生错误行，也不会崩', () => {
    const report = analyzeHookOutput('');

    expect(report.phase).toBe('unknown');
    expect(report.errorCount).toBe(0);
    expect(report.lines).toHaveLength(1);
  });
});
