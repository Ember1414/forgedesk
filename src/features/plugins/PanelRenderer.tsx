import { useTranslation } from 'react-i18next';

import { parsePanelDsl } from '@/features/plugins/panelDsl';
import type { PanelBlock, PanelTextTone } from '@/features/plugins/panelDsl';
import { Button } from '@/ui/components/button';
import { Progress } from '@/ui/components/progress';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/ui/components/table';
import { cn } from '@/lib/utils';

/**
 * 插件面板渲染器（T6.3 方案 C）。
 *
 * 输入是宿主 `render_panel` 返回的 DSL JSON。宿主已校验过一轮；这里再解析
 * 一次并转为强类型块，解析失败渲染**兜底错误卡片**而不是让异常上抛
 * （T6.3 验收：面板渲染失败不能带崩宿主界面）。渲染本身是纯映射——
 * 没有任何插件内容会变成 HTML/属性插值，不存在注入面。
 */

const TONE_CLASS: Record<PanelTextTone, string> = {
  plain: 'text-fg',
  muted: 'text-fg-muted',
  success: 'text-success',
  warning: 'text-warning',
  danger: 'text-danger',
};

function BlockView({
  block,
  onCommand,
}: {
  readonly block: PanelBlock;
  readonly onCommand?: (command: string) => void;
}): React.JSX.Element {
  switch (block.type) {
    case 'heading':
      return <h3 className="text-16 font-semibold text-fg">{block.text}</h3>;
    case 'text':
      return <p className={cn('text-13', TONE_CLASS[block.tone ?? 'plain'])}>{block.text}</p>;
    case 'keyValue':
      return (
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-13">
          {block.entries.map(([key, value]) => (
            // key/value 都是插件给的展示文本，成对出现且可重复，用组合键
            <FragmentKeyed key={`${key}\u0000${value}`} dt={key} dd={value} />
          ))}
        </dl>
      );
    case 'table':
      return (
        <Table>
          <TableHeader>
            <TableRow>
              {block.columns.map((column) => (
                <TableHead key={column}>{column}</TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {block.rows.map((row, rowIndex) => (
              <TableRow key={row.join('\u0000')}>
                {row.map((cell, cellIndex) => (
                  <TableCell key={`${rowIndex}-${cellIndex}`}>{cell}</TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      );
    case 'list':
      return (
        <ul className="list-inside list-disc text-13 text-fg">
          {block.items.map((item) => (
            <li key={item}>{item}</li>
          ))}
        </ul>
      );
    case 'progress':
      return (
        <div>
          <ProgressRow label={block.label} value={block.value} />
        </div>
      );
    case 'button':
      return (
        <Button size="sm" variant="secondary" onClick={() => onCommand?.(block.command)}>
          {block.label}
        </Button>
      );
  }
}

/** keyValue 的单行（dt/dd 必须直接连着 dl，抽成组件让 key 落在包装上）。 */
function FragmentKeyed({
  dt,
  dd,
}: {
  readonly dt: string;
  readonly dd: string;
}): React.JSX.Element {
  return (
    <>
      <dt className="font-medium text-fg-muted">{dt}</dt>
      <dd className="text-fg">{dd}</dd>
    </>
  );
}

function ProgressRow({
  label,
  value,
}: {
  readonly label: string;
  readonly value: number;
}): React.JSX.Element {
  const { t } = useTranslation('shell');
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-13 text-fg">{label}</span>
      <div className="w-40">
        <ProgressBar label={t('plugins.progressLabel', { label })} value={value / 100} />
      </div>
      <span className="font-mono text-12 text-fg-subtle">{value}%</span>
    </div>
  );
}

function ProgressBar({
  label,
  value,
}: {
  readonly label: string;
  readonly value: number;
}): React.JSX.Element {
  return <Progress label={label} value={value} showValue={false} />;
}

/**
 * 面板渲染器。
 *
 * `onCommand`：插件 button 的命令回调——T6.4 管理页接入命令执行链路后由
 * 上层传入；未传入时按钮置灰（渲染契约不依赖命令面板就绪）。
 */
export function PanelRenderer({
  dslJson,
  onCommand,
  className,
}: {
  /** 宿主 `render_panel` 返回的 DSL JSON 文本。 */
  readonly dslJson: string;
  readonly onCommand?: (command: string) => void;
  readonly className?: string;
}): React.JSX.Element {
  const { t } = useTranslation('shell');
  let blocks: readonly PanelBlock[];
  try {
    blocks = parsePanelDsl(dslJson);
  } catch {
    // 宿主校验是第一道门；走到这里说明数据在 IPC 边界被改坏——
    // 展示兜底卡片，绝不把异常抛给上层（T6.3 验收）
    return (
      <div
        role="alert"
        className={cn(
          'flex flex-col gap-1 rounded-lg border border-line bg-surface p-4',
          className,
        )}
        data-testid="plugin-panel-fallback"
      >
        <p className="text-14 font-medium text-fg">{t('plugins.panelRenderFailedTitle')}</p>
        <p className="text-12 text-fg-muted">{t('plugins.panelRenderFailedHint')}</p>
      </div>
    );
  }
  return (
    <div className={cn('flex flex-col gap-3', className)} data-testid="plugin-panel-content">
      {blocks.map((block, index) => (
        <BlockView
          key={`${block.type}-${index}`}
          block={block}
          // exactOptionalPropertyTypes：可选 prop 不得显式传 undefined
          {...(onCommand === undefined ? {} : { onCommand })}
        />
      ))}
    </div>
  );
}
