import { CopyBlock, Details } from '@agent-room/ui-system';
import { useTranslation } from 'react-i18next';

/**
 * 你的账户 ID：别人要邀请你进私人房间时用。产品不按名字或邮箱查账号（那会让任何人枚举成员），
 * 想被邀请的人自己把 ID 交出去，这本身就是同意的信号。
 */
export function AccountIdCopy({ principalId }: { readonly principalId: string }) {
  const { t } = useTranslation();
  return (
    <Details className="account-id-copy" summary={t('rooms.accountId')}>
      <p>{t('rooms.accountId.hint')}</p>
      <CopyBlock
        copiedLabel={t('rooms.accountId.copied')}
        copyLabel={t('rooms.accountId.copy')}
        failedLabel={t('rooms.accountId.copyFailed')}
        size="compact"
        text={principalId}
        textLabel={t('rooms.accountId')}
        tone="quiet"
      />
    </Details>
  );
}
