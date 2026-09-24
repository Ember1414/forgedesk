/**
 * 错误归一化与展示（前端侧的统一入口）。
 *
 * 后端契约：所有跨越 IPC 的错误都必须是 `forgedesk_domain::AppError`，
 * 形状为 `{ code, message, detail?, hint?, actions[], retryable }`
 * （见 docs/API.md 与 crates/domain/src/error.rs）。
 *
 * 但**不能假定**错误一定是那个形状：Tauri 的 `invoke` 在下列情形会抛出别的东西——
 *
 *   - 命令不存在（拼错命令名）：抛字符串 `"Command xxx not found"`；
 *   - 参数反序列化失败：抛字符串或对象；
 *   - 前端自身的 bug（空引用等）：抛 `TypeError` 实例。
 *
 * 因此这里提供 [normalizeError]，把这些形态都收敛成 AppError，
 * 让 UI 永远只需要处理一种错误形状——否则每个 catch 里都要写一遍分支，
 * 而"漏写一个分支"的结果就是用户看到 `[object Object]`。
 */
import { useCallback } from 'react';

import { useTranslation } from 'react-i18next';

import { invokeCommand } from '@/lib/ipc';
import { pushToast } from '@/stores/toastStore';

/**
 * 错误码清单，必须与 Rust 侧 `ErrorCode` 完全一致（只增不改）。
 *
 * 刻意手写而不是从后端拉取：这是**编译期契约**，
 * 若某个码不存在，`errorTitleKey` 会退化为兜底文案；若清单漏了新码，
 * `src/lib/errors.test.ts` 的覆盖率断言会在拿到新错误码时失败。
 */
export const ERROR_CODES = [
  'PATH_NOT_REPO',
  'GIT_CONFLICT',
  'AUTH_REQUIRED',
  'AUTH_EXPIRED',
  'PERMISSION_DENIED',
  'NOT_FOUND',
  'VALIDATION',
  'NETWORK',
  'RATE_LIMITED',
  'PATCH_APPLY_FAILED',
  'PLAN_STALE',
  'HOOK_REJECTED',
  'EMPTY_COMMIT',
  'RESTORE_VERIFY_FAILED',
  'KEYRING_UNAVAILABLE',
  'STORAGE',
  'PTY_UNSUPPORTED',
  'UNSUPPORTED_BY_ENGINE',
  'CANCELLED',
  'INTERNAL',
] as const;

export type ErrorCode = (typeof ERROR_CODES)[number];

/** 后端给出的可点击修复动作。 */
export interface FixAction {
  readonly id: string;
  readonly labelKey: string;
  readonly command: string;
  readonly args?: Record<string, unknown>;
}

/** 归一化后的错误（与 Rust 侧 `AppError` 同形，但字段都有确定值）。 */
export interface NormalizedError {
  readonly code: ErrorCode;
  readonly message: string;
  readonly detail?: string;
  readonly hint?: string;
  readonly actions: readonly FixAction[];
  readonly retryable: boolean;
}

/** 是否为已知错误码。 */
export function isErrorCode(value: unknown): value is ErrorCode {
  return typeof value === 'string' && (ERROR_CODES as readonly string[]).includes(value);
}

/** 后端 `AppError` 的形状判定（宽松：只要求 code 合法）。 */
export function isAppError(value: unknown): boolean {
  return (
    typeof value === 'object' &&
    value !== null &&
    'code' in value &&
    isErrorCode((value as { code: unknown }).code)
  );
}

function readString(source: Record<string, unknown>, key: string): string | undefined {
  const value = source[key];
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

function readActions(source: Record<string, unknown>): readonly FixAction[] {
  const value = source['actions'];
  if (!Array.isArray(value)) {
    return [];
  }
  return value.flatMap((entry) => {
    if (typeof entry !== 'object' || entry === null) {
      return [];
    }
    const candidate = entry as Record<string, unknown>;
    // command 与 labelKey 缺一不可：缺 command 点了没反应，缺 labelKey 会显示裸 key
    if (typeof candidate['command'] !== 'string' || typeof candidate['labelKey'] !== 'string') {
      return [];
    }
    const args = candidate['args'];
    return [
      {
        id: typeof candidate['id'] === 'string' ? candidate['id'] : candidate['command'],
        labelKey: candidate['labelKey'],
        command: candidate['command'],
        ...(typeof args === 'object' && args !== null
          ? { args: args as Record<string, unknown> }
          : {}),
      },
    ];
  });
}

/** 把任意抛出的东西收敛成 [NormalizedError]。 */
export function normalizeError(error: unknown): NormalizedError {
  if (isAppError(error)) {
    const source = error as Record<string, unknown>;
    const code = source['code'] as ErrorCode;
    return {
      code,
      message: readString(source, 'message') ?? '',
      ...(readString(source, 'detail') === undefined
        ? {}
        : { detail: readString(source, 'detail') as string }),
      ...(readString(source, 'hint') === undefined
        ? {}
        : { hint: readString(source, 'hint') as string }),
      actions: readActions(source),
      retryable: source['retryable'] === true,
    };
  }

  // Tauri 的命令不存在 / 参数错误会抛字符串
  if (typeof error === 'string') {
    return {
      code: 'INTERNAL',
      message: error,
      actions: [],
      retryable: false,
    };
  }

  if (error instanceof Error) {
    return {
      code: 'INTERNAL',
      message: error.message,
      actions: [],
      retryable: false,
    };
  }

  // 兜底：不要把对象序列化成 "[object Object]" 丢给用户，而是明确说明"形态未知"
  return {
    code: 'INTERNAL',
    message: typeof error === 'undefined' ? 'unknown error' : JSON.stringify(error),
    actions: [],
    retryable: false,
  };
}

/** 标题 i18n key；未知错误码回落到通用文案。 */
export function errorTitleKey(code: ErrorCode): string {
  return isErrorCode(code) ? `${code}.title` : 'unknownTitle';
}

/** 建议 i18n key；未知错误码回落到通用文案。 */
export function errorHintKey(code: ErrorCode): string {
  return isErrorCode(code) ? `${code}.hint` : 'unknownHint';
}

/**
 * 展示一个错误：转成带详情与动作的 Toast。
 *
 * 为什么放在 hook 里而不是纯函数：文案要走 i18n，而 i18n 语言可能在运行时切换。
 * hook 每次渲染重新取 `t`，保证切换语言后新出现的提示是正确语言。
 */
export function useAppError() {
  const { t } = useTranslation('errors');

  const show = useCallback(
    (raw: unknown): NormalizedError => {
      const error = normalizeError(raw);

      pushToast({
        tone: 'danger',
        title: t(errorTitleKey(error.code)),
        // 后端给了针对性建议就优先用，否则用错误码的兜底建议
        description: error.hint ?? t(errorHintKey(error.code)),
        // 记下发生时间：日志查看器据此高亮"错误发生时间附近的行"
        occurredAt: Date.now(),
        // 错误提示不自动消失：一闪而过的错误等于没提示
        duration: 0,
        // detail 与 actions 交给 Toaster 渲染（可折叠 + 可点击）
        ...(error.detail === undefined ? {} : { detail: error.detail }),
        ...(error.actions.length === 0
          ? {}
          : {
              actions: error.actions.map((action) => ({
                id: action.id,
                label: t(action.labelKey),
                command: action.command,
                ...(action.args === undefined ? {} : { args: action.args }),
              })),
            }),
      });

      return error;
    },
    [t],
  );

  /** 执行一个修复动作：成功与失败都给反馈（静默执行会让用户以为按钮坏了）。 */
  const runAction = useCallback(
    async (action: {
      label: string;
      command: string;
      args?: Record<string, unknown>;
    }): Promise<void> => {
      try {
        await invokeCommand(action.command, action.args);
        pushToast({ tone: 'success', title: t('actionDone', { command: action.label }) });
      } catch (error) {
        show(error);
      }
    },
    [show, t],
  );

  return { normalize: normalizeError, show, runAction };
}
