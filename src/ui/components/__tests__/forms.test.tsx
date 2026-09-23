import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { Checkbox } from '@/ui/components/checkbox';
import { Input } from '@/ui/components/input';
import { RadioGroup } from '@/ui/components/radio-group';
import {
  Select,
  SelectContent,
  SelectField,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/ui/components/select';
import { Slider } from '@/ui/components/slider';
import { Switch } from '@/ui/components/switch';
import { Textarea } from '@/ui/components/textarea';

/**
 * 表单控件的测试重点：
 *   - label 与控件必须真正关联（label/for 或 aria-label），否则点标签没反应、读屏软件读不出名称；
 *   - 错误态必须落到 `aria-invalid`，视觉与无障碍语义不能两套；
 *   - 三态、禁用态、键盘步进这些"看起来能用但用不了"的细节。
 */
describe('Input', () => {
  it('label 与输入框通过 for/id 关联，可用 getByLabelText 定位', () => {
    render(<Input label="仓库路径" defaultValue="/tmp/repo" />);
    expect(screen.getByLabelText('仓库路径')).toHaveValue('/tmp/repo');
  });

  it('错误态置 aria-invalid 并用 aria-describedby 关联错误文案', () => {
    render(<Input label="远端地址" error="地址不可达" hint="应能访问" />);

    const input = screen.getByLabelText('远端地址');
    expect(input).toHaveAttribute('aria-invalid', 'true');

    const describedBy = input.getAttribute('aria-describedby');
    expect(describedBy).not.toBeNull();
    expect(document.getElementById(describedBy ?? '')).toHaveTextContent('地址不可达');
  });

  it('禁用态不可编辑', () => {
    render(<Input label="禁用" disabled />);
    expect(screen.getByLabelText('禁用')).toBeDisabled();
  });
});

describe('Textarea', () => {
  it('显示字数统计并遵守 maxLength', () => {
    function Harness() {
      const [value, setValue] = useState('abc');
      return (
        <Textarea
          label="提交信息"
          value={value}
          maxLength={10}
          showCount
          onChange={(event) => {
            setValue(event.target.value);
          }}
        />
      );
    }
    render(<Harness />);

    const textarea = screen.getByLabelText('提交信息');
    expect(screen.getByText('3 / 10')).toBeInTheDocument();

    fireEvent.change(textarea, { target: { value: 'abcdef' } });
    expect(screen.getByText('6 / 10')).toBeInTheDocument();
    expect(textarea).toHaveAttribute('maxlength', '10');
  });

  it('错误态置 aria-invalid', () => {
    render(<Textarea label="说明" error="太长" />);
    expect(screen.getByLabelText('说明')).toHaveAttribute('aria-invalid', 'true');
  });
});

describe('Checkbox', () => {
  it('点击切换选中态', () => {
    const onCheckedChange = vi.fn();
    render(<Checkbox label="暂存此文件" onCheckedChange={onCheckedChange} />);

    fireEvent.click(screen.getByRole('checkbox', { name: '暂存此文件' }));
    expect(onCheckedChange).toHaveBeenCalledWith(true);
  });

  it('支持三态（部分选中以 aria-checked="mixed" 表达）', () => {
    render(<Checkbox label="全选" checked="indeterminate" />);
    expect(screen.getByRole('checkbox', { name: '全选' })).toHaveAttribute('aria-checked', 'mixed');
  });

  it('禁用态不可点击', () => {
    const onCheckedChange = vi.fn();
    render(<Checkbox label="禁用" disabled onCheckedChange={onCheckedChange} />);

    fireEvent.click(screen.getByRole('checkbox', { name: '禁用' }));
    expect(onCheckedChange).not.toHaveBeenCalled();
  });
});

describe('Switch', () => {
  it('点击切换并有 role=switch', () => {
    const onCheckedChange = vi.fn();
    render(<Switch label="自动获取" onCheckedChange={onCheckedChange} />);

    fireEvent.click(screen.getByRole('switch', { name: '自动获取' }));
    expect(onCheckedChange).toHaveBeenCalledWith(true);
  });

  it('禁用态不可切换', () => {
    const onCheckedChange = vi.fn();
    render(<Switch label="禁用" disabled onCheckedChange={onCheckedChange} />);

    fireEvent.click(screen.getByRole('switch', { name: '禁用' }));
    expect(onCheckedChange).not.toHaveBeenCalled();
  });
});

describe('RadioGroup', () => {
  it('点击选择选项', () => {
    const onValueChange = vi.fn();
    render(
      <RadioGroup
        label="快照策略"
        onValueChange={onValueChange}
        options={[
          { value: 'always', label: '总是快照' },
          { value: 'never', label: '不建快照' },
        ]}
      />,
    );

    fireEvent.click(screen.getByRole('radio', { name: '不建快照' }));
    expect(onValueChange).toHaveBeenCalledWith('never');
  });

  it('方向键在组内移动焦点（roving tabindex）', async () => {
    render(
      <RadioGroup
        label="快照策略"
        defaultValue="always"
        options={[
          { value: 'always', label: '总是快照' },
          { value: 'never', label: '不建快照' },
        ]}
      />,
    );

    const first = screen.getByRole('radio', { name: '总是快照' });
    first.focus();
    fireEvent.keyDown(first, { key: 'ArrowDown' });

    // Radix 的 roving focus 会把焦点移到下一项；选中与否由焦点驱动的 onFocus 处理决定，
    // 因此这里断言"焦点确实移动了"——只断言"按键没报错"是无效断言。
    await waitFor(() => {
      expect(screen.getByRole('radio', { name: '不建快照' })).toHaveFocus();
    });
  });

  it('选项可单独禁用', () => {
    render(
      <RadioGroup
        label="快照策略"
        options={[{ value: 'never', label: '不建快照', disabled: true }]}
      />,
    );
    expect(screen.getByRole('radio', { name: '不建快照' })).toBeDisabled();
  });
});

describe('Slider', () => {
  it('渲染当前值，并暴露 aria-valuenow', () => {
    render(<Slider label="并发任务数" value={[3]} min={1} max={8} />);

    const slider = screen.getByRole('slider', { name: '并发任务数' });
    expect(slider).toHaveAttribute('aria-valuenow', '3');
    expect(screen.getByText('3')).toBeInTheDocument();
  });

  it('方向键调整数值', () => {
    const onValueChange = vi.fn();
    render(<Slider label="并发任务数" value={[3]} min={1} max={8} onValueChange={onValueChange} />);

    fireEvent.keyDown(screen.getByRole('slider', { name: '并发任务数' }), { key: 'ArrowRight' });
    expect(onValueChange).toHaveBeenCalledWith([4]);
  });

  it('可自定义值格式化', () => {
    render(<Slider label="并发" value={[2]} formatValue={(value) => `${String(value)} 个`} />);
    expect(screen.getByText('2 个')).toBeInTheDocument();
  });
});

describe('Select', () => {
  it('SelectField 的标签与触发器关联', () => {
    render(
      <SelectField label="远端" value="origin" options={[{ value: 'origin', label: 'origin' }]} />,
    );
    expect(screen.getByRole('combobox', { name: '远端' })).toBeInTheDocument();
  });

  it('禁用时触发器不可用', () => {
    render(<SelectField label="远端" value={undefined} disabled options={[{ value: 'origin' }]} />);
    expect(screen.getByRole('combobox', { name: '远端' })).toBeDisabled();
  });

  it('点击选项后回传选中的值', async () => {
    const onValueChange = vi.fn();
    render(
      <Select onValueChange={onValueChange}>
        <SelectTrigger aria-label="远端">
          <SelectValue placeholder="选择远端" />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="origin">origin</SelectItem>
          <SelectItem value="upstream">upstream</SelectItem>
        </SelectContent>
      </Select>,
    );

    const trigger = screen.getByRole('combobox', { name: '远端' });
    // Radix Select 在鼠标按下时打开（真实浏览器行为），jsdom 需要显式派发 pointerdown
    fireEvent.pointerDown(trigger, { pointerId: 1, pointerType: 'mouse', button: 0 });
    fireEvent.click(trigger);

    fireEvent.click(await screen.findByRole('option', { name: 'upstream' }));
    expect(onValueChange).toHaveBeenCalledWith('upstream');
  });
});
