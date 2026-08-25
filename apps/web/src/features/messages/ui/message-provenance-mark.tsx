import { Bot } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { MessageProvenance } from '@/features/messages/domain/message';

export type MessageProvenanceMarkProps = {
  readonly provenance: MessageProvenance;
};

export function MessageProvenanceMark({ provenance }: MessageProvenanceMarkProps) {
  const { t } = useTranslation();
  if (provenance !== 'autonomous_agent') {
    return null;
  }
  return (
    <span
      aria-label={t('messages.provenance.autonomous_agent')}
      className="message-provenance-mark"
      role="status"
    >
      <Bot aria-hidden="true" />
      <span aria-hidden="true">{t('messages.provenance.autoBadge')}</span>
    </span>
  );
}
