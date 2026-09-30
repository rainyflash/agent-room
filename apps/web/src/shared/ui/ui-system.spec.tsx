// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import {
  Banner,
  CopyBlock,
  Details,
  Dialog,
  Field,
  Segmented,
  Spinner,
  Toast,
  ToastStack,
} from '@agent-room/ui-system';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function DialogHarness({ onClose }: { readonly onClose?: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        onClick={() => {
          setOpen(true);
        }}
        type="button"
      >
        Open
      </button>
      {open ? (
        <Dialog
          closeLabel="Close"
          description="It joins the public lobby."
          onClose={() => {
            onClose?.();
            setOpen(false);
          }}
          title="Bring an agent"
        >
          <button type="button">Inside</button>
        </Dialog>
      ) : null}
    </>
  );
}

describe('Dialog', () => {
  it('挂到 body 上，以标题命名、以说明描述；关闭按钮和 Esc 都走 onClose', () => {
    const onClose = vi.fn();
    const { container } = render(<DialogHarness onClose={onClose} />);
    fireEvent.click(screen.getByRole('button', { name: 'Open' }));
    const dialog = screen.getByRole('dialog', { name: 'Bring an agent' });
    expect(dialog).toHaveAccessibleDescription('It joins the public lobby.');
    expect(container).not.toContainElement(dialog);
    expect(dialog.parentElement).toBe(document.body);

    fireEvent(dialog, new Event('cancel', { cancelable: true }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('dialog')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Open' }));
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(onClose).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('关掉后焦点回到打开它的按钮', () => {
    render(<DialogHarness />);
    const trigger = screen.getByRole('button', { name: 'Open' });
    trigger.focus();
    fireEvent.click(trigger);
    screen.getByRole('button', { name: 'Inside' }).focus();
    expect(trigger).not.toHaveFocus();
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(trigger).toHaveFocus();
  });
});

function SegmentedHarness({ onChange }: { readonly onChange: (value: string) => void }) {
  const [value, setValue] = useState<'network' | 'mcp' | 'cli'>('network');
  return (
    <Segmented
      label="How your agent connects"
      onChange={(next) => {
        onChange(next);
        setValue(next);
      }}
      options={[
        { value: 'network', label: 'Network' },
        { value: 'mcp', label: 'MCP' },
        { value: 'cli', label: 'Command line' },
      ]}
      value={value}
    />
  );
}

describe('Segmented', () => {
  it('是一个带名字的单选组，只有选中的那个在 Tab 顺序里', () => {
    render(<SegmentedHarness onChange={vi.fn()} />);
    expect(screen.getByRole('radiogroup', { name: 'How your agent connects' })).toBeVisible();
    const radios = screen.getAllByRole('radio');
    expect(radios.map((radio) => radio.getAttribute('aria-checked'))).toEqual([
      'true',
      'false',
      'false',
    ]);
    expect(radios.map((radio) => radio.tabIndex)).toEqual([0, -1, -1]);
    fireEvent.click(screen.getByRole('radio', { name: 'Command line' }));
    expect(screen.getByRole('radio', { name: 'Command line' })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    expect(screen.getAllByRole('radio').map((radio) => radio.tabIndex)).toEqual([-1, -1, 0]);
  });

  it('方向键在选项间循环移动并选中，Home / End 到头尾', () => {
    const onChange = vi.fn<(value: string) => void>();
    render(<SegmentedHarness onChange={onChange} />);
    const network = screen.getByRole('radio', { name: 'Network' });
    network.focus();
    fireEvent.keyDown(network, { key: 'ArrowLeft' });
    expect(screen.getByRole('radio', { name: 'Command line' })).toHaveFocus();
    fireEvent.keyDown(document.activeElement ?? network, { key: 'ArrowRight' });
    expect(screen.getByRole('radio', { name: 'Network' })).toHaveAttribute('aria-checked', 'true');
    fireEvent.keyDown(document.activeElement ?? network, { key: 'End' });
    fireEvent.keyDown(document.activeElement ?? network, { key: 'Home' });
    fireEvent.keyDown(document.activeElement ?? network, { key: 'ArrowDown' });
    expect(onChange.mock.calls.map(([value]) => value)).toEqual([
      'cli',
      'network',
      'cli',
      'network',
      'mcp',
    ]);
    // 别的键不管。
    fireEvent.keyDown(document.activeElement ?? network, { key: 'a' });
    expect(onChange).toHaveBeenCalledTimes(5);
  });
});

describe('Banner', () => {
  it('危险用 alert 播报，其余用 status；可以不当实时区域', () => {
    render(
      <>
        <Banner tone="danger">Stopped</Banner>
        <Banner tone="warning" title="Authorize">
          Allow this computer
        </Banner>
        <Banner role={null} tone="info">
          Quiet note
        </Banner>
      </>,
    );
    expect(screen.getByRole('alert')).toHaveTextContent('Stopped');
    expect(screen.getByRole('status')).toHaveTextContent('AuthorizeAllow this computer');
    expect(screen.getByText('Quiet note').closest('.ar-banner')).not.toHaveAttribute('role');
  });

  it('操作放在右侧；图标可以换掉或去掉', () => {
    render(
      <Banner action={<button type="button">Retry</button>} icon={null} tone="warning">
        Reconnecting
      </Banner>,
    );
    const banner = screen.getByRole('status');
    expect(banner.querySelector('.ar-banner__icon')).toBeNull();
    expect(banner.querySelector('.ar-banner__action')).toContainElement(
      screen.getByRole('button', { name: 'Retry' }),
    );
  });
});

describe('Details', () => {
  it('用原生 details，默认收起，可以一开始就展开', () => {
    render(
      <>
        <Details summary="First time using MCP?">Setup</Details>
        <Details defaultOpen summary="Details">
          fixture.code
        </Details>
      </>,
    );
    const closed = screen.getByText('First time using MCP?').closest('details');
    const open = screen.getByText('Details').closest('details');
    expect(closed).not.toHaveAttribute('open');
    expect(open).toHaveAttribute('open');
  });
});

describe('Field', () => {
  it('标签、说明和错误都关联到输入框上；有错误时标成无效', () => {
    const { rerender } = render(
      <Field hint="Up to 50." label="Invite people">
        <textarea />
      </Field>,
    );
    const input = screen.getByRole('textbox', { name: 'Invite people' });
    expect(input).toHaveAccessibleDescription('Up to 50.');
    expect(input).not.toHaveAttribute('aria-invalid');

    rerender(
      <Field error="Each line must be an account ID." hint="Up to 50." label="Invite people">
        <textarea />
      </Field>,
    );
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(input).toHaveAccessibleDescription('Up to 50. Each line must be an account ID.');
    expect(screen.getByRole('alert')).toHaveTextContent('Each line must be an account ID.');
  });

  it('输入框自己带的 id 保留下来', () => {
    render(
      <Field label="Name">
        <input id="room-name" />
      </Field>,
    );
    expect(screen.getByRole('textbox', { name: 'Name' })).toHaveAttribute('id', 'room-name');
  });
});

describe('Spinner', () => {
  it('没有文字时只是装饰；有文字时读屏读出来', () => {
    const { container } = render(<Spinner />);
    expect(container.querySelector('.ar-spinner')).toHaveAttribute('aria-hidden', 'true');
    expect(screen.queryByRole('status')).toBeNull();
    cleanup();
    render(<Spinner label="Loading" />);
    expect(screen.getByRole('status', { name: 'Loading' })).toBeVisible();
  });
});

describe('CopyBlock', () => {
  const labels = {
    copiedLabel: 'Copied. Send it to your agent.',
    copyLabel: 'Copy message',
    failedLabel: 'Couldn’t copy.',
    textLabel: 'Message for your agent',
  };

  it('复制成功后按钮换成“已复制”；原文改了就回到初始状态', async () => {
    const writeText = vi.fn(() => Promise.resolve());
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const onCopied = vi.fn();
    const view = render(<CopyBlock {...labels} onCopied={onCopied} text="join --room Studio" />);
    expect(screen.getByLabelText('Message for your agent')).toHaveTextContent('join --room Studio');
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
      await Promise.resolve();
    });
    expect(writeText).toHaveBeenCalledWith('join --room Studio');
    expect(onCopied).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('button', { name: 'Copied. Send it to your agent.' })).toBeVisible();

    view.rerender(<CopyBlock {...labels} onCopied={onCopied} text="join --room Lobby" />);
    expect(screen.getByRole('button', { name: 'Copy message' })).toBeVisible();
  });

  it('复制失败时说明可以自己选中复制，不报成功', async () => {
    vi.stubGlobal('navigator', {
      clipboard: { writeText: () => Promise.reject(new Error('denied')) },
    });
    const onCopied = vi.fn();
    render(<CopyBlock {...labels} onCopied={onCopied} text="join" />);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
      await Promise.resolve();
    });
    expect(await screen.findByRole('status')).toHaveTextContent('Couldn’t copy.');
    expect(onCopied).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Copy message' })).toBeVisible();
  });

  it('还不能发时按钮灰着但原文照样显示；可以只留按钮', () => {
    const { rerender } = render(<CopyBlock {...labels} disabled text="join" />);
    expect(screen.getByRole('button', { name: 'Copy message' })).toBeDisabled();
    expect(screen.getByLabelText('Message for your agent')).toBeVisible();
    rerender(<CopyBlock {...labels} hideText text="join" />);
    expect(screen.queryByLabelText('Message for your agent')).toBeNull();
    expect(screen.getByRole('button', { name: 'Copy message' })).toBeEnabled();
  });
});

describe('Toast', () => {
  it('提示栈是一个带名字的区域；一条提示可以带操作，也可以关掉', () => {
    const onDismiss = vi.fn();
    render(
      <ToastStack label="Notifications">
        <Toast
          action={<button type="button">Install</button>}
          dismissLabel="Later"
          onDismiss={onDismiss}
          title="Agent Room 0.2.0 is ready to install"
        />
        <Toast role="alert" title="Another device wants to verify" tone="warning" />
      </ToastStack>,
    );
    const stack = screen.getByRole('region', { name: 'Notifications' });
    expect(within(stack).getByRole('status')).toHaveTextContent(
      'Agent Room 0.2.0 is ready to install',
    );
    expect(within(stack).getByRole('alert')).toHaveTextContent('Another device wants to verify');
    expect(within(stack).getByRole('button', { name: 'Install' })).toBeVisible();
    fireEvent.click(within(stack).getByRole('button', { name: 'Later' }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('没给关闭回调就没有关闭按钮', () => {
    render(<Toast title="Quiet toast" />);
    expect(screen.queryByRole('button')).toBeNull();
  });
});
