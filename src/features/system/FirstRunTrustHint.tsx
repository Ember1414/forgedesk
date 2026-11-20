import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import { isTauriRuntime, systemOpenUrl } from '@/lib/ipc';
import {
  DEFAULT_TRUST_HINT_DISMISSED,
  TRUST_HINT_DISMISSED_KEY,
  useSettingsStore,
} from '@/stores/settingsStore';
import { Button } from '@/ui/components/button';

/**
 * 首次启动的「安装可信性」提示（M8 / T8.2）。
 *
 * # 为什么要有它
 *
 * 零成本方案下应用没有付费代码签名：macOS 的 Gatekeeper 会拦截（极端时直接说
 * "已损坏"），Windows 的 SmartScreen 会提示"未知发布者"。用户此刻最容易的反应是
 * "这软件有问题"然后放弃——与其等他去翻仓库文档，不如在他**已经选择继续、第一次
 * 成功打开应用**时坦白原因，并给出可自行验证的路径（校验和 / GPG 签名）。
 *
 * # 展示与关闭
 *
 * 只在首次启动展示；「我知道了」把 `trustHint.dismissed` 写入设置，之后不再出现。
 * 不做"每次启动都提示"：一条永远消不掉的提示会被学会无视（与 UpdateBanner 同一取舍）。
 * 设置 → 隐私 里提供「重新显示」入口，误关不需要重装。
 *
 * # 为什么不是对话框
 *
 * 引导信息不该阻塞第一次使用：用户此刻最该做的事是打开他的仓库，而不是先读完提示。
 * 卡片出现在内容区上方，读不读都不影响操作。
 */

/**
 * 校验指引地址：官网下载页。
 *
 * 用官网而不是 GitHub 仓库里的文档：下载页按访问者平台给出对应的校验命令，
 * 且与应用内的更新清单同源（同一份发布数据），不会出现"文档写的哈希与发布不一致"。
 */
const TRUST_VERIFY_URL = 'https://forgedesk.pages.dev/download';

export function FirstRunTrustHint() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const inTauri = isTauriRuntime();

  const settingsLoaded = useSettingsStore((state) => state.loaded);
  const setJson = useSettingsStore((state) => state.setJson);
  // 选择器里调 `getJson`（而非拿函数引用）：在设置页点「重新显示」（写回 false）时，
  // 这条卡片能立刻回来（zustand 只在选择器结果变化时重渲染，见 settingsStore 说明）。
  const dismissed = useSettingsStore((state) =>
    state.getJson<boolean>(TRUST_HINT_DISMISSED_KEY, DEFAULT_TRUST_HINT_DISMISSED),
  );

  // 未运行在 Tauri 宿主（浏览器预览）或设置尚未加载完成时一律不显示：
  // 否则会出现"先闪一下再消失"或把开发预览当成真实首次启动。
  if (!inTauri || !settingsLoaded || dismissed) {
    return null;
  }

  return (
    <div role="note" className="border-b border-line bg-surface px-3 py-2.5">
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
        <div className="min-w-0 max-w-3xl">
          <p className="text-13 font-medium">{t('trustHint.title')}</p>
          <p className="mt-0.5 text-12 text-fg-muted">{t('trustHint.body')}</p>
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              // 打开指引后卡片保留：用户可能还要回来点「我知道了」，
              // 强制关闭会让人以为提示"消失了又出现"
              void systemOpenUrl(TRUST_VERIFY_URL).catch(show);
            }}
          >
            {t('trustHint.verifyGuide')}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              // 乐观更新：点击后卡片立刻消失；写入失败时 store 回滚、卡片回来并弹错误
              void setJson(TRUST_HINT_DISMISSED_KEY, true).catch(show);
            }}
          >
            {t('trustHint.gotIt')}
          </Button>
        </div>
      </div>
    </div>
  );
}
