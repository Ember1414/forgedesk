// i18n-ignore-file
// 本文件是开发专用页面（路由只在 dev 构建注册），文案面向开发者，
// 因此整文件豁免 i18n:lint；正式界面一律走 i18n key。
// 注意：该标记必须出现在文件前 6 行内（检查脚本只扫文件头部，避免被误用于某个中间片段）。
import { useState } from 'react';
import type { ReactNode } from 'react';

import { Info, Plus, Trash2 } from 'lucide-react';

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/ui/components/alert-dialog';
import { Badge } from '@/ui/components/badge';
import { Button } from '@/ui/components/button';
import { Checkbox } from '@/ui/components/checkbox';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from '@/ui/components/context-menu';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/ui/components/dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/ui/components/dropdown-menu';
import { EmptyState } from '@/ui/components/empty-state';
import { ErrorState } from '@/ui/components/error-state';
import { IconButton } from '@/ui/components/icon-button';
import { Input } from '@/ui/components/input';
import { Popover, PopoverContent, PopoverTrigger } from '@/ui/components/popover';
import { Progress } from '@/ui/components/progress';
import { RadioGroup } from '@/ui/components/radio-group';
import { SelectField } from '@/ui/components/select';
import {
  Sheet,
  SheetBody,
  SheetContent,
  SheetDescription,
  SheetTitle,
  SheetTrigger,
} from '@/ui/components/sheet';
import { SkeletonText } from '@/ui/components/skeleton';
import { Slider } from '@/ui/components/slider';
import { SplitPane } from '@/ui/components/split-pane';
import { Switch } from '@/ui/components/switch';
import {
  Table,
  TableBody,
  TableCell,
  TableEmptyRow,
  TableHead,
  TableHeader,
  TableRow,
  TableSkeletonRows,
} from '@/ui/components/table';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/ui/components/tabs';
import { Tag } from '@/ui/components/tag';
import { Textarea } from '@/ui/components/textarea';
import { ToggleGroup } from '@/ui/components/toggle-group';
import { Tip } from '@/ui/components/tooltip';
import { VirtualList } from '@/ui/components/virtual-list';
import { debugPanic, debugThrowError, isTauriRuntime } from '@/lib/ipc';
import { useAppError } from '@/lib/errors';
import { openLogViewer } from '@/stores/logViewerStore';
import { pushToast } from '@/stores/toastStore';
import { useUiStore } from '@/stores/uiStore';

/**
 * 组件展示页（开发专用，路由 /__dev__/components 只在 dev 构建注册）。
 *
 * 目的：把所有组件 × 全部状态放在一屏可滚动的页面里。
 * 组件库最常见的问题不是"写不出来"，而是"某个状态忘了做样式"——
 * 例如只写了默认态没有禁用态、只写了浅色没有暗色。逐状态摊开能一眼看出来。
 *
 * 本页是开发页面、不进生产构建，因此文案直接用中文（同 DesignSystemPage 的约定，
 * T6.7 的 i18n:lint 会把它列入白名单）。
 */

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="rounded-lg border border-line bg-surface p-5 shadow-sm">
      <h2 className="text-16 font-semibold">{title}</h2>
      {description !== undefined ? (
        <p className="mt-1 text-12 text-fg-subtle">{description}</p>
      ) : null}
      <div className="mt-4 flex flex-col gap-4">{children}</div>
    </section>
  );
}

function Row({ children }: { children: ReactNode }) {
  return <div className="flex flex-wrap items-center gap-2">{children}</div>;
}

interface DemoRow {
  readonly id: string;
  readonly name: string;
  readonly size: number;
}

const DEMO_ROWS: readonly DemoRow[] = Array.from({ length: 500 }, (_, index) => ({
  id: `row-${String(index)}`,
  name: `src/features/repo/file-${String(index)}.ts`,
  size: (index % 40) + 1,
}));

/** 错误链路演示用的代表性错误码（覆盖认证 / 网络 / 仓库状态 / 校验几类）。 */
const DEMO_ERROR_CODES = [
  'PATH_NOT_REPO',
  'GIT_CONFLICT',
  'AUTH_REQUIRED',
  'NETWORK',
  'RATE_LIMITED',
  'VALIDATION',
] as const;

export function ComponentsPage() {
  const themeMode = useUiStore((state) => state.themeMode);
  const setThemeMode = useUiStore((state) => state.setThemeMode);
  const { show } = useAppError();
  const [sort, setSort] = useState<'asc' | 'desc'>('asc');
  const [checkboxState, setCheckboxState] = useState<boolean | 'indeterminate'>('indeterminate');
  const [sliderValue, setSliderValue] = useState<number[]>([3]);
  const [selectedTags, setSelectedTags] = useState<string[]>(['后端']);

  const sortedRows = [...DEMO_ROWS].sort((left, right) =>
    sort === 'asc' ? left.size - right.size : right.size - left.size,
  );

  return (
    <main className="h-full overflow-y-auto bg-canvas">
      <div className="mx-auto flex max-w-5xl flex-col gap-5 p-6">
        <header>
          <h1 className="text-24 font-semibold tracking-tight">ForgeDesk 组件库</h1>
          <p className="mt-1 text-13 text-fg-muted">
            T0.5 · 基于 Radix 原语 · 颜色仅用语义 token · 全部组件支持键盘与暗色主题
          </p>
        </header>

        <Section title="按钮 Button" description="5 种变体 × 3 档尺寸，含 loading 与 disabled">
          <Row>
            {(['primary', 'secondary', 'ghost', 'danger', 'link'] as const).map((variant) => (
              <Button key={variant} variant={variant}>
                {variant}
              </Button>
            ))}
          </Row>
          <Row>
            <Button size="sm">sm</Button>
            <Button size="md">md</Button>
            <Button size="lg">lg</Button>
            <Button loading>提交中…</Button>
            <Button disabled>不可用</Button>
            <Button variant="danger" loading>
              危险 + 加载
            </Button>
          </Row>
        </Section>

        <Section title="图标按钮 IconButton" description="aria-label 必填，可挂 Tooltip 补充说明">
          <Row>
            <IconButton label="新增" variant="ghost">
              <Plus aria-hidden="true" className="size-4" />
            </IconButton>
            <IconButton label="删除" variant="danger">
              <Trash2 aria-hidden="true" className="size-4" />
            </IconButton>
            <IconButton label="不可用" variant="secondary" disabled>
              <Info aria-hidden="true" className="size-4" />
            </IconButton>
            <Tip content="悬停提示：仅作补充，不能是唯一标签来源">
              <IconButton label="提示演示" variant="secondary">
                <Info aria-hidden="true" className="size-4" />
              </IconButton>
            </Tip>
          </Row>
        </Section>

        <Section title="表单控件" description="默认 / 错误 / 禁用 / 三态 / 只读值">
          <div className="grid gap-4 md:grid-cols-2">
            <Input label="仓库路径" placeholder="E:\\Projects\\example" />
            <Input
              label="远端地址"
              defaultValue="https://example.com/a/b.git"
              error="地址不可达"
              hint="应能访问"
            />
            <Input label="禁用的输入框" defaultValue="不可编辑" disabled />
            <Textarea
              label="提交信息"
              placeholder="一句话说清楚改了什么"
              maxLength={72}
              showCount
            />
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            <SelectField
              label="远端"
              value="origin"
              placeholder="选择远端"
              options={[
                { value: 'origin', label: 'origin' },
                { value: 'upstream', label: 'upstream' },
              ]}
            />
            <SelectField
              label="禁用的选择器"
              value={undefined}
              placeholder="先打开仓库"
              disabled
              options={[{ value: 'a' }]}
            />
          </div>

          <Row>
            <Checkbox label="普通复选" checked onChange={() => undefined} />
            <Checkbox
              label="半选（部分暂存）"
              checked={checkboxState}
              onCheckedChange={(next) => {
                setCheckboxState(next === 'indeterminate' ? false : next);
              }}
            />
            <Checkbox label="禁用" disabled />
          </Row>

          <RadioGroup
            label="快照策略"
            defaultValue="always"
            options={[
              {
                value: 'always',
                label: '每次写操作前都建快照',
                description: '最安全，磁盘占用略高',
              },
              { value: 'destructive', label: '仅破坏性操作前建快照' },
              { value: 'never', label: '不建快照', description: '不推荐', disabled: true },
            ]}
          />

          <Row>
            <Switch label="自动获取远程更新" defaultChecked />
            <Switch label="禁用的开关" disabled />
          </Row>

          <Slider
            label="并发任务数"
            value={sliderValue}
            min={1}
            max={8}
            step={1}
            onValueChange={setSliderValue}
            formatValue={(current) => `${String(current)} 个`}
          />
        </Section>

        <Section
          title="浮层"
          description="Dialog / AlertDialog（影响说明必填）/ Sheet / Popover / 菜单"
        >
          <Row>
            <Dialog>
              <DialogTrigger asChild>
                <Button variant="secondary">打开对话框</Button>
              </DialogTrigger>
              <DialogContent closeLabel="关闭">
                <DialogHeader>
                  <DialogTitle>对话框标题</DialogTitle>
                  <DialogDescription>
                    焦点会被限制在对话框内，Esc 关闭后焦点回到触发按钮。
                  </DialogDescription>
                </DialogHeader>
                <DialogFooter>
                  <Button variant="secondary">取消</Button>
                  <Button>确定</Button>
                </DialogFooter>
              </DialogContent>
            </Dialog>

            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button variant="danger">删除分支（确认框）</Button>
              </AlertDialogTrigger>
              <AlertDialogContent
                impactLabel="影响说明"
                impact={
                  <>
                    分支 <span className="font-mono">feature/login</span> 的 3 个提交将不再有引用，
                    30 天后可能被垃圾回收。执行前会自动创建快照。
                  </>
                }
              >
                <AlertDialogHeader>
                  <AlertDialogTitle>删除本地分支？</AlertDialogTitle>
                  <AlertDialogDescription>
                    此操作不可撤销，但可以从快照回滚。
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>取消</AlertDialogCancel>
                  <AlertDialogAction>删除分支</AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>

            <Sheet>
              <SheetTrigger asChild>
                <Button variant="secondary">打开抽屉</Button>
              </SheetTrigger>
              <SheetContent side="right" closeLabel="关闭">
                <SheetTitle className="text-16 font-semibold">提交详情</SheetTitle>
                <SheetDescription className="text-12 text-fg-subtle">
                  抽屉不打断流程，适合"边看边操作"。
                </SheetDescription>
                <SheetBody>
                  <SkeletonText lines={6} />
                </SheetBody>
              </SheetContent>
            </Sheet>

            <Popover>
              <PopoverTrigger asChild>
                <Button variant="secondary">浮层面板</Button>
              </PopoverTrigger>
              <PopoverContent>
                <p className="text-13">非模态浮层：不锁焦点，点别处即可继续。</p>
              </PopoverContent>
            </Popover>

            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="secondary">下拉菜单</Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent aria-label="演示菜单">
                <DropdownMenuLabel>分支操作</DropdownMenuLabel>
                <DropdownMenuItem shortcut="⌘C">复制分支名</DropdownMenuItem>
                <DropdownMenuItem>检出此分支</DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem destructive shortcut="⌫">
                  删除分支
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>

            <ContextMenu>
              <ContextMenuTrigger asChild>
                <div className="rounded-md border border-dashed border-line px-3 py-1.5 text-13 text-fg-muted">
                  在此区域右键
                </div>
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuItem>复制路径</ContextMenuItem>
                <ContextMenuItem>在编辑器中打开</ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem destructive>丢弃此文件的改动</ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          </Row>
        </Section>

        <Section title="导航与切换" description="Tabs（方向键移动）/ ToggleGroup（多选一）">
          <Tabs defaultValue="changes">
            <TabsList>
              <TabsTrigger value="changes" count={12}>
                变更
              </TabsTrigger>
              <TabsTrigger value="staged" count={3}>
                已暂存
              </TabsTrigger>
              <TabsTrigger value="history">历史</TabsTrigger>
              <TabsTrigger value="disabled" disabled>
                不可用
              </TabsTrigger>
            </TabsList>
            <TabsContent value="changes">
              <p className="text-13 text-fg-muted">12 个变更（示例内容）</p>
            </TabsContent>
            <TabsContent value="staged">
              <p className="text-13 text-fg-muted">3 个已暂存（示例内容）</p>
            </TabsContent>
            <TabsContent value="history">
              <p className="text-13 text-fg-muted">历史（示例内容）</p>
            </TabsContent>
          </Tabs>

          <Row>
            <ToggleGroup
              label="主题"
              value={themeMode}
              options={[
                { value: 'light', label: '亮色' },
                { value: 'dark', label: '暗色' },
                { value: 'system', label: '跟随系统' },
              ]}
              onValueChange={(next) => {
                setThemeMode(next === 'dark' ? 'dark' : next === 'light' ? 'light' : 'system');
              }}
            />
            <ToggleGroup
              label="禁用示例"
              value="a"
              options={[
                { value: 'a', label: '可用' },
                { value: 'b', label: '不可用', disabled: true },
              ]}
              onValueChange={() => undefined}
            />
          </Row>
        </Section>

        <Section title="数据展示" description="Table（排序 / 空态 / 骨架）与 VirtualList（500 行）">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead widthClassName="w-2/3">文件</TableHead>
                <TableHead
                  widthClassName="w-32"
                  sortDirection={sort}
                  onSort={() => {
                    setSort((previous) => (previous === 'asc' ? 'desc' : 'asc'));
                  }}
                >
                  改动行数
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {sortedRows.slice(0, 5).map((row) => (
                <TableRow key={row.id}>
                  <TableCell className="font-mono text-12">{row.name}</TableCell>
                  <TableCell className="text-right font-mono text-12">+{row.size}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>

          <div className="grid gap-4 md:grid-cols-2">
            <div className="rounded-md border border-line p-2">
              <p className="mb-1 text-12 text-fg-subtle">空态</p>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>文件</TableHead>
                    <TableHead>状态</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  <TableEmptyRow colSpan={2}>工作区干净，没有待提交的改动</TableEmptyRow>
                </TableBody>
              </Table>
            </div>
            <div className="rounded-md border border-line p-2">
              <p className="mb-1 text-12 text-fg-subtle">加载态</p>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>文件</TableHead>
                    <TableHead>状态</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  <TableSkeletonRows rows={3} columns={2} />
                </TableBody>
              </Table>
            </div>
          </div>

          <VirtualList
            label="虚拟列表演示（500 行）"
            items={sortedRows}
            itemHeight={28}
            height={196}
            getKey={(row) => row.id}
            renderItem={(row, index) => (
              <div className="flex h-full items-center justify-between gap-2 px-2 text-12 odd:bg-surface-sunken">
                <span className="truncate font-mono">{row.name}</span>
                <span className="shrink-0 text-fg-subtle">#{index}</span>
              </div>
            )}
          />
        </Section>

        <Section
          title="反馈"
          description="Toast（队列）/ Progress / Skeleton / Badge / Tag / 空态 / 错误态"
        >
          <Row>
            <Button
              variant="secondary"
              onClick={() => {
                pushToast({ tone: 'info', title: '正在获取远程更新' });
              }}
            >
              信息提示
            </Button>
            <Button
              variant="secondary"
              onClick={() => {
                pushToast({
                  tone: 'success',
                  title: '推送完成',
                  description: 'main → origin/main',
                });
              }}
            >
              成功提示
            </Button>
            <Button
              variant="secondary"
              onClick={() => {
                pushToast({ tone: 'warning', title: '快照空间接近上限', duration: 0 });
              }}
            >
              警告提示（不自动消失）
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                pushToast({
                  tone: 'danger',
                  title: '推送被拒绝',
                  description: '远端有新的提交，请先拉取。',
                  detail: 'remote: rejected\nstatus: non-fast-forward',
                  duration: 0,
                  actions: [
                    {
                      id: 'retry',
                      label: '重试',
                      onClick: () => {
                        pushToast({ tone: 'info', title: '正在重试…' });
                      },
                    },
                  ],
                });
              }}
            >
              错误提示（含详情与动作）
            </Button>
          </Row>

          <div className="grid gap-4 md:grid-cols-2">
            <Progress label="确定进度" value={0.42} hint="3 / 8 个对象" />
            <Progress label="不确定进度" value={null} hint="等待远端返回总量" />
          </div>

          <SkeletonText lines={3} />

          <Row>
            <Badge tone="neutral">默认</Badge>
            <Badge tone="brand">当前分支</Badge>
            <Badge tone="success">已通过</Badge>
            <Badge tone="warning">需注意</Badge>
            <Badge tone="danger">失败</Badge>
            <Badge tone="info">提示</Badge>
          </Row>

          <Row>
            {['后端', '前端', '文档'].map((tag) => (
              <Tag
                key={tag}
                selected={selectedTags.includes(tag)}
                onClick={() => {
                  setSelectedTags((previous) =>
                    previous.includes(tag)
                      ? previous.filter((item) => item !== tag)
                      : [...previous, tag],
                  );
                }}
              >
                {tag}
              </Tag>
            ))}
            <Tag
              onRemove={() => {
                pushToast({ tone: 'info', title: '已移除标签' });
              }}
              removeLabel="移除标签 临时"
            >
              临时
            </Tag>
          </Row>

          <div className="grid gap-4 md:grid-cols-2">
            <EmptyState
              title="还没有打开的仓库"
              description="打开一个本地仓库或克隆一个远端仓库后，这里会显示最近记录。"
              action={
                <Button size="sm">
                  <Plus aria-hidden="true" className="size-3.5" />
                  打开仓库
                </Button>
              }
              footnote="git clone <url>"
            />
            <ErrorState
              title="无法读取仓库状态"
              hint="目录可能不是 Git 仓库，或当前用户没有读取权限。"
              details={'fatal: not a git repository (or any of the parent directories): .git'}
              retryLabel="重试"
              onRetry={() => {
                pushToast({ tone: 'info', title: '正在重试…' });
              }}
            />
          </div>
        </Section>

        <Section
          title="错误链路（AppError）"
          description="触发真实的后端错误：分类 → 脱敏 → IPC → i18n → Toast（含可折叠详情与可点击动作）"
        >
          <Row>
            {DEMO_ERROR_CODES.map((code) => (
              <Button
                key={code}
                variant="secondary"
                onClick={() => {
                  if (isTauriRuntime()) {
                    // 真实链路：命令返回 AppError，前端归一化后展示
                    void debugThrowError(code).catch(show);
                    return;
                  }
                  // 浏览器预览（无 Tauri 宿主）：本地构造同形状的错误，
                  // 这样在没有桌面宿主时也能审视"标题/建议/详情/动作"的排版
                  show({
                    code,
                    message: 'browser preview',
                    detail:
                      'remote: https://alice:ghp_DEMO0000000000000000000000000000@example.com rejected\n(浏览器预览模式：详情由后端脱敏，这里仅示意)',
                    retryable: false,
                    actions: [
                      { id: 'demo.refresh', labelKey: 'actions.refresh', command: 'app_version' },
                    ],
                  });
                }}
              >
                {code}
              </Button>
            ))}
          </Row>
          <p className="text-12 text-fg-subtle">
            说明：详情里的假令牌应显示为 <span className="font-mono">ghp_«redacted»</span>
            ，若能看到完整令牌说明脱敏层失效（红线 R8）。
          </p>

          <Row>
            <Button
              variant="danger"
              onClick={() => {
                if (!isTauriRuntime()) {
                  show({
                    code: 'UNSUPPORTED_BY_ENGINE',
                    message: 'browser preview',
                    retryable: false,
                  });
                  return;
                }
                // 触发真实 panic：验证 panic 日志、会话标记与"界面不受影响"
                void debugPanic();
              }}
            >
              触发 panic（T0.8 验收）
            </Button>
            <Button
              variant="secondary"
              onClick={() => {
                openLogViewer({ nearTimestamp: Date.now() });
              }}
            >
              打开日志查看器
            </Button>
          </Row>
          <p className="text-12 text-fg-subtle">
            触发 panic 后：① 后台线程崩溃、界面继续可用；② 日志目录出现
            <span className="font-mono"> panic-&lt;时间戳&gt;.log</span>； ③ 不清除{' '}
            <span className="font-mono">session.lock</span> 直接关掉应用，
            下次启动会在日志里看到"上次会话未正常退出"（M7 会据此提供恢复引导）。
          </p>
        </Section>

        <Section title="布局" description="SplitPane（拖拽或方向键调整）与 Resizable">
          <div className="h-56">
            <SplitPane
              separatorLabel="调整左侧宽度"
              defaultPrimarySize={220}
              primary={
                <div className="h-full overflow-auto rounded-md border border-line bg-surface-sunken p-2 text-12">
                  主区（可拖拽右边缘，或聚焦分隔条后用 ←/→ 调整）
                </div>
              }
              secondary={
                <div className="h-full overflow-auto rounded-md border border-line bg-surface p-2 text-12">
                  副区自动占满剩余空间
                </div>
              }
            />
          </div>
        </Section>
      </div>
    </main>
  );
}
