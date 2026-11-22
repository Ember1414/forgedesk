/** 不依赖 IPC 的错误描述：错误路径自身不能因循环引用、BigInt 或序列化失败而崩溃。 */
export function describeError(error: unknown): { message: string; stack?: string } {
  if (error instanceof Error) {
    return error.stack === undefined
      ? { message: error.message }
      : { message: error.message, stack: error.stack };
  }
  if (typeof error === 'string') return { message: error };
  try {
    return { message: JSON.stringify(error) ?? String(error) };
  } catch {
    try {
      const text = String(error);
      return { message: text === '[object Object]' ? 'unserializable error object' : text };
    } catch {
      return { message: 'unserializable error' };
    }
  }
}
