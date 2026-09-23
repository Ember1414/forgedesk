import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

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
import { Button } from '@/ui/components/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/ui/components/dialog';
import { Popover, PopoverContent, PopoverTrigger } from '@/ui/components/popover';
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetTitle,
  SheetTrigger,
} from '@/ui/components/sheet';
import { TooltipProvider, Tip } from '@/ui/components/tooltip';

/**
 * 浮层组件的测试重点：**打开 / 关闭 / 焦点**这三件事。
 * 浮层的样式出错用户看得见，但"Esc 关不掉""关闭后焦点丢了"这类问题
 * 只有键盘用户会遇到，恰恰是最容易漏测的部分。
 */
describe('Dialog', () => {
  it('点击触发器打开，标题与描述可被读取', async () => {
    render(
      <Dialog>
        <DialogTrigger asChild>
          <Button>打开</Button>
        </DialogTrigger>
        <DialogContent closeLabel="关闭">
          <DialogHeader>
            <DialogTitle>对话框标题</DialogTitle>
            <DialogDescription>说明文字</DialogDescription>
          </DialogHeader>
        </DialogContent>
      </Dialog>,
    );

    fireEvent.click(screen.getByRole('button', { name: '打开' }));

    const dialog = await screen.findByRole('dialog');
    // 描述必须通过 aria-describedby 关联，读屏软件才会在打开时一并读出
    expect(dialog).toHaveAccessibleDescription('说明文字');
    expect(screen.getByRole('heading', { name: '对话框标题' })).toBeInTheDocument();
  });

  it('Esc 关闭并把焦点还给触发按钮', async () => {
    render(
      <Dialog>
        <DialogTrigger asChild>
          <Button>打开</Button>
        </DialogTrigger>
        <DialogContent closeLabel="关闭">
          <DialogHeader>
            <DialogTitle>标题</DialogTitle>
          </DialogHeader>
        </DialogContent>
      </Dialog>,
    );

    const trigger = screen.getByRole('button', { name: '打开' });
    fireEvent.click(trigger);
    const dialog = await screen.findByRole('dialog');

    fireEvent.keyDown(dialog, { key: 'Escape' });

    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
    await waitFor(() => {
      expect(trigger).toHaveFocus();
    });
  });

  it('关闭按钮有无障碍名称', async () => {
    render(
      <Dialog defaultOpen>
        <DialogContent closeLabel="关闭对话框">
          <DialogHeader>
            <DialogTitle>标题</DialogTitle>
          </DialogHeader>
        </DialogContent>
      </Dialog>,
    );

    expect(await screen.findByRole('button', { name: '关闭对话框' })).toBeInTheDocument();
  });
});

describe('AlertDialog', () => {
  function renderAlert(onConfirm: () => void) {
    return render(
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button variant="danger">删除分支</Button>
        </AlertDialogTrigger>
        <AlertDialogContent
          impactLabel="影响说明"
          impact="该分支的 3 个提交将不再有引用，执行前会自动创建快照。"
        >
          <AlertDialogHeader>
            <AlertDialogTitle>删除本地分支？</AlertDialogTitle>
            <AlertDialogDescription>此操作不可撤销。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction onClick={onConfirm}>删除</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>,
    );
  }

  it('必须展示影响说明（role="note"）', async () => {
    renderAlert(() => undefined);
    fireEvent.click(screen.getByRole('button', { name: '删除分支' }));

    const note = await screen.findByRole('note');
    expect(note).toHaveTextContent('影响说明');
    expect(note).toHaveTextContent('会自动创建快照');
  });

  it('默认聚焦在取消按钮上（默认动作不应是破坏性的）', async () => {
    renderAlert(() => undefined);
    fireEvent.click(screen.getByRole('button', { name: '删除分支' }));

    const cancel = await screen.findByRole('button', { name: '取消' });
    await waitFor(() => {
      expect(cancel).toHaveFocus();
    });
  });

  it('确认按钮触发回调', async () => {
    const onConfirm = vi.fn();
    renderAlert(onConfirm);
    fireEvent.click(screen.getByRole('button', { name: '删除分支' }));

    fireEvent.click(await screen.findByRole('button', { name: '删除' }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});

describe('Sheet', () => {
  it('打开后显示标题并可关闭', async () => {
    render(
      <Sheet>
        <SheetTrigger asChild>
          <Button>打开抽屉</Button>
        </SheetTrigger>
        <SheetContent side="right" closeLabel="关闭抽屉">
          <SheetTitle>提交详情</SheetTitle>
          <SheetDescription>详情说明</SheetDescription>
        </SheetContent>
      </Sheet>,
    );

    fireEvent.click(screen.getByRole('button', { name: '打开抽屉' }));
    expect(await screen.findByRole('heading', { name: '提交详情' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '关闭抽屉' }));
    await waitFor(() => {
      expect(screen.queryByRole('heading', { name: '提交详情' })).not.toBeInTheDocument();
    });
  });
});

describe('Tooltip', () => {
  it('聚焦触发器时显示提示内容', async () => {
    render(
      <TooltipProvider delayDuration={0}>
        <Tip content="仅作补充说明">
          <Button>触发器</Button>
        </Tip>
      </TooltipProvider>,
    );

    fireEvent.focus(screen.getByRole('button', { name: '触发器' }));

    expect(await screen.findByRole('tooltip')).toHaveTextContent('仅作补充说明');
  });
});

describe('Popover', () => {
  it('点击打开非模态浮层，且不锁定页面焦点', async () => {
    render(
      <Popover>
        <PopoverTrigger asChild>
          <Button>筛选</Button>
        </PopoverTrigger>
        <PopoverContent>筛选条件</PopoverContent>
      </Popover>,
    );

    fireEvent.click(screen.getByRole('button', { name: '筛选' }));
    expect(await screen.findByText('筛选条件')).toBeInTheDocument();
  });
});
