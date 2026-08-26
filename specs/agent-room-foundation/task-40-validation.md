# 任务 40 验证记录：生产部署与伸缩路径

## 1. 当前结论

生产 Compose、严格配置模型、Secret 文件注入、一次性迁移/建桶、外部依赖模式、控制平面副本和 Synapse Worker 路径已经实现并在 Docker Linux 容器中完成真实启动验证。

任务 40 的最终验收仍为 **No-Go**：本机是 Windows + Docker Desktop，不是干净 Linux 主机；示例使用保留域名，因此没有执行真实 ACME、公网健康检查和 Matrix 联邦委派。同时任务 40 的前置任务 36、39 尚未完成。不能把容器内验证伪装成公网生产验收。

## 2. 已实现边界

- `infra/production/compose.yaml` 覆盖 Caddy、Synapse、Keycloak、控制平面、PostgreSQL、SeaweedFS、ClamAV、OTel 和 Redis；
- `deployment.schema.json` 与 Python 领域解析器共同拒绝未知字段、不安全 TLS、重复域名和越界副本；
- 所有应用 Secret 支持 `_FILE`，直接值与文件同时出现时立即失败；
- PostgreSQL 迁移角色与运行角色分离，运行角色权限由迁移显式授予；
- 首次安装自动生成并保留 Synapse signing key，外部对象存储只核验预建桶；
- 控制平面 1–16 副本与 Synapse 0–8 Worker 使用同一配置事实源渲染；
- 主机预检、安装、升级、健康、联邦诊断与停止均由 `tools/production.py` 自动执行；
- 没有引入 Kubernetes。

## 3. 真实容器证据

2026-08-25 在 Docker Desktop Linux Engine 上执行了以下验证：

1. 生产 Keycloak、Web/Caddy 和 Rust 控制平面镜像完成冷构建；
2. PostgreSQL 18、SeaweedFS、ClamAV、Keycloak、Synapse 与控制平面真实启动；
3. 一次性对象桶初始化成功，第二次执行保持幂等；
4. 数据库迁移容器成功执行全部迁移；
5. 控制平面 `/health/ready` 返回 PostgreSQL、Matrix、对象存储全部 `ready`；
6. 将配置提升为 2 个控制平面副本、Redis 和 2 个 Synapse Generic Worker 后，全部实例健康；
7. 实际启动过程抓出并修复了未加引号 `tmpfs`、只读目录嵌套挂载、内部服务名安全校验和 Worker 网络隔离问题。

关键就绪响应：

```json
{
  "status": "ready",
  "dependencies": [
    { "name": "postgresql", "status": "ready" },
    { "name": "matrix", "status": "ready" },
    { "name": "object_store", "status": "ready" }
  ]
}
```

## 4. 自动门禁

```text
cargo test -p agent-room-content-adapter --all-features
  14 通过，3 个真实依赖测试按设计忽略

cargo test -p agent-room-control-plane --all-features
  71 通过，2 个真实依赖测试按设计忽略

cargo test -p agent-room-matrix-adapter -p agent-room-matrix-provisioning-adapter --all-features
  40 通过，6 个外部服务测试按设计忽略

python -m unittest tools.tests.test_prodops -v
  11 通过

python tools/production.py validate --config infra/production/deployment.example.json ...
python tools/production.py validate --config infra/production/deployment.external.example.json ...
  两套 Compose 均通过真实 docker compose 解析
```

此外，生成的 Caddyfile 由固定版本 Caddy 容器验证，Synapse 主进程与 Worker 配置由固定版本 Synapse 镜像解析。

## 5. 未满足门禁

- 干净 Linux 主机从空目录执行自动安装；
- 五个真实 DNS、ACME 证书、公开健康检查和外部 Matrix 联邦版本入口；
- 外部 PostgreSQL/S3 凭据与网络策略的真实运营环境联调；
- 任务 36 的封闭测试证据与任务 39 的长时间容量/Bridge 证据。

在上述证据齐备前，`tasks.md` 中任务 40 保持未勾选。
