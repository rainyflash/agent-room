import { Laptop, MonitorSmartphone } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { FleetDevice } from '@/features/workspace/domain/agent-fleet';
import { formatWorkspaceTime } from '@/features/workspace/ui/workspace-format';

export function DeviceRail({ devices }: { readonly devices: readonly FleetDevice[] }) {
  const { i18n, t } = useTranslation();
  return (
    <aside className="workspace-device-rail">
      <header>
        <MonitorSmartphone aria-hidden="true" />
        <h2>{t('workspace.deviceRail.title')}</h2>
        <span>{devices.length.toString().padStart(2, '0')}</span>
      </header>
      {devices.length === 0 ? (
        <div className="workspace-device-rail__empty">
          <Laptop aria-hidden="true" />
          <p>{t('workspace.deviceRail.empty')}</p>
        </div>
      ) : (
        <ol>
          {devices.map((device) => (
            <li data-current={device.current ? 'true' : 'false'} key={device.deviceId}>
              <Laptop aria-hidden="true" />
              <div>
                <strong>{device.label}</strong>
                <span>{t(`security.access.platform.${device.platform}`)}</span>
                <small>
                  {t('workspace.deviceRail.instances', { count: device.instanceCount })}
                </small>
                <time
                  dateTime={new Date(
                    device.lastSeenAtUnixMs ?? device.createdAtUnixMs,
                  ).toISOString()}
                >
                  {t(
                    device.lastSeenAtUnixMs === null
                      ? 'workspace.deviceRail.registered'
                      : 'workspace.deviceRail.lastSeen',
                    {
                      time: formatWorkspaceTime(
                        device.lastSeenAtUnixMs ?? device.createdAtUnixMs,
                        i18n.resolvedLanguage,
                      ),
                    },
                  )}
                </time>
              </div>
              {device.current ? <span>{t('workspace.deviceRail.current')}</span> : null}
            </li>
          ))}
        </ol>
      )}
    </aside>
  );
}
