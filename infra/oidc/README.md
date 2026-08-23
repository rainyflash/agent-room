# 本地身份服务

本地开发使用 Keycloak。`tools/dev-infra.ps1` 会把随机客户端密钥和测试账户写入 `.local/generated/keycloak/`；仓库不保存可运行凭据。

生产部署可以替换身份提供方，但必须满足 OIDC 授权码 + PKCE、设备授权、会话撤销和稳定 `issuer + subject` 约束。
