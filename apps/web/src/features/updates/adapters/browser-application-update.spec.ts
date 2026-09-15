// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyWebApplicationUpdate, waitForWorkerInstallation } from './browser-application-update';
import { registerUpdateGuard } from '../application/update-readiness';

class InstallingWorker extends EventTarget {
  state: ServiceWorkerState = 'installing';
  change(state: ServiceWorkerState) {
    this.state = state;
    this.dispatchEvent(new Event('statechange'));
  }
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('网页升级缓存生命周期', () => {
  it('更新文件尚未缓存完成时不激活旧 worker，安装完成后再切换', async () => {
    const worker = new InstallingWorker();
    const update = vi.fn(() => Promise.resolve());
    vi.stubGlobal('navigator', {
      serviceWorker: {
        getRegistration: () =>
          Promise.resolve({
            update,
            installing: worker,
            get waiting() {
              return worker.state === 'installed' ? worker : null;
            },
          }),
      },
    });
    const activate = vi.fn(() => Promise.resolve());
    const applying = applyWebApplicationUpdate(activate);
    await vi.waitFor(() => {
      expect(update).toHaveBeenCalledOnce();
    });
    expect(activate).not.toHaveBeenCalled();
    worker.change('installed');
    await applying;
    expect(activate).toHaveBeenCalledOnce();
  });

  it('缓存失败或超时显示失败，不把未安装的版本当作已就绪', async () => {
    const failed = new InstallingWorker();
    const pendingFailure = waitForWorkerInstallation(failed);
    failed.change('redundant');
    await expect(pendingFailure).rejects.toThrow('update.install_failed');
    vi.useFakeTimers();
    const pendingTimeout = waitForWorkerInstallation(new InstallingWorker());
    const rejected = expect(pendingTimeout).rejects.toThrow('update.install_timeout');
    await vi.advanceTimersByTimeAsync(30_000);
    await rejected;
  });

  it('附件尚未保存时拒绝升级，不丢失当前草稿', async () => {
    const detach = registerUpdateGuard(() => false);
    const activate = vi.fn(() => Promise.resolve());
    try {
      await expect(applyWebApplicationUpdate(activate)).rejects.toThrow('update.draft_unsaved');
      expect(activate).not.toHaveBeenCalled();
    } finally {
      detach();
    }
  });
});
