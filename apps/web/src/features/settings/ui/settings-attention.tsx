import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { useOptionalAppServices } from '@/app/app-services';
import { useDeviceSigningState } from '@/features/security/data/device-signing-state';
import { matrixSecurityQueryOptions } from '@/features/security/data/matrix-security-queries';
import { thisDeviceState } from '@/features/security/domain/device-signing';
import type { MatrixSecurityGateway } from '@/features/security/domain/matrix-security';
import { useOptionalSession } from '@/features/session/ui/session-provider';
import type { DeviceSigningStatus } from '@/shared/matrix/device-signing-status';

/** 安全状态在顶栏上不用刷得太勤：一分钟看一次，安全页里照旧实时。 */
const ATTENTION_STALE_MS = 60_000;

/**
 * “设置”上的提醒点：这台设备的自动签名出错了（要点“重试”），或者桌面端有新版本可装。
 * 读屏读出原因。没连上 Matrix 时不查安全状态（查也查不到）。
 */
export function SettingsAttentionDot({ updateReady }: { readonly updateReady: boolean }) {
  const services = useOptionalAppServices();
  const session = useOptionalSession();
  const connected = session?.snapshot.context.connection != null;
  if (services === null || !connected) {
    return updateReady ? <AttentionDot reason="update" /> : null;
  }
  return (
    <ConnectedAttentionDot
      deviceSigning={services.deviceSigning}
      gateway={services.security}
      updateReady={updateReady}
    />
  );
}

/** 设置页“安全”分节上的提醒点：只看这台设备的自动签名有没有出错。 */
export function SecurityAttentionDot() {
  const services = useOptionalAppServices();
  const session = useOptionalSession();
  const connected = session?.snapshot.context.connection != null;
  if (services === null || !connected) return null;
  return (
    <ConnectedAttentionDot
      deviceSigning={services.deviceSigning}
      gateway={services.security}
      updateReady={false}
    />
  );
}

function ConnectedAttentionDot({
  deviceSigning,
  gateway,
  updateReady,
}: {
  readonly deviceSigning: DeviceSigningStatus;
  readonly gateway: MatrixSecurityGateway;
  readonly updateReady: boolean;
}) {
  const inspection = useQuery({
    ...matrixSecurityQueryOptions(gateway),
    staleTime: ATTENTION_STALE_MS,
  });
  const signing = useDeviceSigningState(deviceSigning);
  const snapshot = inspection.data?.ok === true ? inspection.data.value : null;
  if (thisDeviceState(snapshot, signing) === 'failed') return <AttentionDot reason="signing" />;
  return updateReady ? <AttentionDot reason="update" /> : null;
}

function AttentionDot({ reason }: { readonly reason: 'signing' | 'update' }) {
  const { t } = useTranslation();
  return (
    <span className="app-navigation__dot" data-reason={reason}>
      <span className="sr-only">
        {t(reason === 'signing' ? 'security.attention' : 'settings.updateDot')}
      </span>
    </span>
  );
}
