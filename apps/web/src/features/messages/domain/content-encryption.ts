export type ClientContentEncryption = {
  readonly algorithm: 'io.github.rainyflash.agentroom.content.aes-256-gcm.v1';
  readonly contextId: string;
  readonly keyBase64Url: string;
  readonly nonceBase64Url: string;
  readonly plaintextSizeBytes: number;
};
