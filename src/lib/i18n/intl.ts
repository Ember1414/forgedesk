/**
 * Intl 统一封装（T6.7）。
 *
 * # 为什么禁止散落的 toLocaleString()
 *
 * `Date.prototype.toLocaleString()` 不带参数时使用**运行环境默认 locale**，
 * 与应用选择的语言无关——中文界面里可能混出英文日期（反之亦然）。
 * 这里集中提供按 `i18n.resolvedLanguage` 格式化的函数，调用方不再自己选 locale。
 *
 * 相对时间走 `Intl.RelativeTimeFormat`（"3 minutes ago" vs "3 分钟前"
 * 的语法差异由引擎处理），替代手写模板插值。
 */
import i18n from '@/lib/i18n';

/** 当前界面的 BCP 47 语言标签。 */
function locale(): string {
  return i18n.resolvedLanguage ?? i18n.language ?? 'en-US';
}

/** 绝对日期时间（列表/详情用；不含秒以下精度）。 */
export function formatDateTime(value: Date | number | null | undefined): string | null {
  if (
    value === null ||
    value === undefined ||
    !Number.isFinite(typeof value === 'number' ? value : value.getTime())
  ) {
    return null;
  }
  return new Intl.DateTimeFormat(locale(), {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(value);
}

/** 仅日期（无时间部分）。 */
export function formatDate(value: Date | number | null | undefined): string | null {
  if (
    value === null ||
    value === undefined ||
    !Number.isFinite(typeof value === 'number' ? value : value.getTime())
  ) {
    return null;
  }
  return new Intl.DateTimeFormat(locale(), { dateStyle: 'medium' }).format(value);
}

type RelativeUnit = 'second' | 'minute' | 'hour' | 'day' | 'month' | 'year';

/** 把**带方向**的秒差解析成 (带符号数值, 单位)——RelativeTimeFormat：
 *  正数 = 未来，负数 = 过去。分档用绝对值，符号原样保留。 */
function splitRelative(deltaSeconds: number): [number, RelativeUnit] {
  const sign = deltaSeconds < 0 ? -1 : 1;
  const seconds = Math.abs(deltaSeconds);
  if (seconds < 60) {
    return [sign * seconds, 'second'];
  }
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return [sign * minutes, 'minute'];
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return [sign * hours, 'hour'];
  }
  const days = Math.floor(hours / 24);
  if (days < 31) {
    return [sign * days, 'day'];
  }
  const months = Math.floor(days / 30.44);
  if (months < 12) {
    return [sign * months, 'month'];
  }
  return [sign * Math.floor(months / 12), 'year'];
}

/**
 * 相对时间（"3 分钟前" / "in 3 minutes"）。
 *
 * 未来时间与过去时间同一路径（RelativeTimeFormat 自带方向）；
 * 60 秒内统一显示"刚刚/now"级别的文案由调用方按需判断。
 */
export function formatRelative(targetSeconds: number, nowSeconds: number): string {
  const delta = targetSeconds - nowSeconds; // 正 = 未来，负 = 过去（RelativeTimeFormat 的约定）
  const [count, unit] = splitRelative(delta);
  // RelativeTimeFormat：正数 = 未来（"in 3 days"），负数 = 过去（"3 days ago"）
  return new Intl.RelativeTimeFormat(locale(), { numeric: 'auto' }).format(count, unit);
}
