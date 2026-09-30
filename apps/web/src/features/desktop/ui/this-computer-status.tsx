import { Link } from '@tanstack/react-router';
import { Monitor } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { thisComputerState } from '../domain/desktop-connection';
import { useOptionalDesktopRuntimeController } from './desktop-runtime-provider';
import './this-computer.css';

/** 顶栏右侧的小状态：这台电脑连没连上。点开到“我的 Agent”里的“这台电脑”一节。只在桌面端显示。 */
export function ThisComputerStatus() {
  const { t } = useTranslation();
  const controller = useOptionalDesktopRuntimeController();
  if (controller?.available !== true) return null;
  const state = thisComputerState(controller.snapshot?.bridge);
  const label = t(`thisComputer.state.${state}`);
  return (
    <Link
      aria-label={t('thisComputer.status', { state: label })}
      className="this-computer-status"
      data-state={state}
      hash="this-computer"
      search={{}}
      title={t('thisComputer.status', { state: label })}
      to="/workspace"
    >
      <Monitor aria-hidden="true" />
      <span aria-hidden="true" className="this-computer-status__dot" />
      <span className="this-computer-status__label">{label}</span>
    </Link>
  );
}
