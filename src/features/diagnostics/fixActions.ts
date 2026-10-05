/**
 * 修复动作执行器（T5.6）。
 *
 * # 三档动作的安全边界
 *
 * - `command`：**白名单**内只读/安全命令直接执行（fetch、set-upstream、stash）；
 *   白名单外的 command 一律拒绝执行并提示——诊断数据可能来自运行时覆盖目录，
 *   不能让它任意调用 IPC 命令（零信任）。
 * - `guide`：页面标识（`/...` 路由）经 `onNavigate` 跳转；URL 交给系统浏览器。
 * - `dangerous`：**绝不在这里执行**——回调 `onDangerous` 交给容器打开
 *   DangerousActionDialog（计划预览 + 快照说明 + 确认）。
 *
 * 执行结果：成功 → 刷新（调用方注入的 invalidate 回调）；失败 → 走错误 Toast
 * 并对新错误递归诊断（由 useAppError.show + toast 集成天然完成）。
 */
import { gitFetch, gitPush, gitStashSave, invokeCommand } from '@/lib/ipc';
import type { DiagFix, DiagFixAction } from '@/lib/ipc';

/** 诊断修复允许直接执行的命令白名单（全部是安全/可回滚动作）。 */
export const SAFE_FIX_COMMANDS = new Set([
  'git_fetch',
  'git_push', // 仅当 args 只含 setUpstream（force_with_lease 走 dangerous 档）
  'git_stash_save',
]);

export interface FixExecutionContext {
  readonly repoId: number;
  /** 动作成功后刷新仓库相关查询。 */
  readonly invalidate: () => void;
  /** kind=dangerous 的动作转交容器打开确认对话框。 */
  readonly onDangerous: (fix: DiagFix) => void;
  /** kind=guide 的页面跳转。 */
  readonly onNavigate: (route: string) => void;
  /** 执行结果反馈（成功文案）。 */
  readonly onDone: (message: string) => void;
}

/**
 * 执行一个**已经过用户确认**的危险修复动作（DangerousActionDialog 的确认回调）。
 *
 * 目前唯一的危险动作是 force-with-lease 强推（R7 允许的形态）；未来扩充时
 * 在这里按 fix.id 分派，仍然不允许任何"未确认"的路径到达这里。
 */
export async function runDangerousFix(fix: DiagFix, context: FixExecutionContext): Promise<void> {
  if (fix.action.kind !== 'dangerous') {
    return;
  }
  if (fix.action.command === 'git_push') {
    await gitPush(context.repoId, { forceWithLease: true });
    context.invalidate();
    context.onDone(fix.id);
    return;
  }
  // 未映射的危险动作：显式失败（不会静默跳过）
  throw new Error(`diag.action.unmappedDangerous:${fix.id}`);
}

/**
 * 判断一个 command 动作是否真的安全（白名单 + 参数审计）：
 * `git_push` 只有在 `set_upstream`（等价于 push 前的关联建立）时才放行。
 */
export function isSafeCommandAction(action: DiagFixAction): boolean {
  if (action.kind !== 'command' || action.command === undefined) {
    return false;
  }
  if (action.command === 'git_push') {
    return action.args !== undefined && action.args['set_upstream'] === true;
  }
  return SAFE_FIX_COMMANDS.has(action.command);
}

/** 执行一个修复动作（按 kind 分派）。 */
export async function runFixAction(fix: DiagFix, context: FixExecutionContext): Promise<void> {
  const { action } = fix;
  switch (action.kind) {
    case 'command': {
      if (!isSafeCommandAction(action)) {
        // 防御：覆盖目录注入了一个白名单外的 command——按 guide 缺席处理，
        // 给用户一个明确提示而不是静默失败。
        throw new Error(`diag.action.notAllowed:${action.command ?? '(none)'}`);
      }
      const args = (action.args ?? {}) as Record<string, unknown>;
      switch (action.command) {
        case 'git_fetch':
          await gitFetch(context.repoId, {
            ...(typeof args['remote'] === 'string' ? { remote: args['remote'] as string } : {}),
          });
          break;
        case 'git_push':
          await gitPush(context.repoId, { setUpstream: true });
          break;
        case 'git_stash_save':
          await gitStashSave(context.repoId, { includeUntracked: true });
          break;
        default:
          // 白名单新增时忘了写分派：显式失败而不是假装成功
          throw new Error(`diag.action.unmapped:${action.command}`);
      }
      context.invalidate();
      context.onDone(fix.id);
      return;
    }
    case 'guide': {
      if (action.value === undefined) {
        return;
      }
      if (action.value.startsWith('/')) {
        context.onNavigate(action.value);
      } else {
        // URL：交给系统浏览器（后端 open_url 做协议白名单校验）
        await invokeCommand('system_open_url', { url: action.value });
      }
      return;
    }
    case 'dangerous':
      // 绝不在这里执行：转交 DangerousActionDialog，确认后走 runDangerousFix
      context.onDangerous(fix);
      return;
    default:
      // 未知 kind：宁可不动也不猜
      return;
  }
}
