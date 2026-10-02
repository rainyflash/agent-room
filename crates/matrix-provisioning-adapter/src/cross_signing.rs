//! 人的设备自动签名重建签名身份时（ADR 0011），替这个人上传新的签名公钥。
//!
//! 已有签名身份时换身份，Synapse 要交互认证；没接 MAS 的部署里，管理员开的“免认证换身份”
//! 豁免不起作用，只有应用服务的请求能免（MSC4190）。所以由控制面以应用服务的身份、冒充这个
//! 本地用户上传。这要求应用服务注册里有一个覆盖本地用户的非独占命名空间
//! （`tools/prodops/render.py`）。私钥只在人的设备上，这里传的只有公钥和签名。

use agent_room_application::ports::{
    MatrixCrossSigningKeys, MatrixCrossSigningResetGateway, MatrixFailure, MatrixFailureKind,
    MatrixOperation, MatrixResult, MatrixUserId, PortFuture,
};

use crate::{
    MatrixApplicationServiceProvisioner, decode_matrix_error, map_matrix_error,
    map_transport_error, read_limited_body,
};

impl MatrixApplicationServiceProvisioner {
    async fn replace_cross_signing_keys_internal(
        &self,
        user_id: &MatrixUserId,
        keys: &MatrixCrossSigningKeys,
    ) -> MatrixResult<()> {
        let operation = MatrixOperation::ReplaceCrossSigningKeys;
        self.ensure_local_user(user_id, operation)?;
        let mut endpoint =
            self.endpoint("_matrix/client/v3/keys/device_signing/upload", operation)?;
        endpoint
            .query_pairs_mut()
            .append_pair("user_id", user_id.as_str());
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(self.access_token.expose())
            .json(keys.as_json())
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if status.is_success() {
            return Ok(());
        }
        Err(map_matrix_error(
            operation,
            status,
            &decode_matrix_error(&body, operation)?,
        ))
    }

    /// 只替本服务器上的用户上传；别的服务器的用户应用服务本来也冒充不了。
    fn ensure_local_user(
        &self,
        user_id: &MatrixUserId,
        operation: MatrixOperation,
    ) -> MatrixResult<()> {
        let suffix = format!(":{}", self.server_name);
        let localpart = user_id
            .as_str()
            .strip_prefix('@')
            .and_then(|value| value.strip_suffix(&suffix));
        if localpart.is_none_or(str::is_empty) {
            return Err(MatrixFailure::new(operation, MatrixFailureKind::Forbidden));
        }
        Ok(())
    }
}

impl MatrixCrossSigningResetGateway for MatrixApplicationServiceProvisioner {
    fn replace_cross_signing_keys<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
        keys: &'a MatrixCrossSigningKeys,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(self.replace_cross_signing_keys_internal(user_id, keys))
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use agent_room_application::ports::{
        MatrixCrossSigningKeys, MatrixCrossSigningResetGateway, MatrixFailureKind, MatrixUserId,
        SecretValue,
    };
    use axum::{
        Json, Router,
        extract::{Query, State},
        http::{HeaderMap, StatusCode, header::AUTHORIZATION},
        response::IntoResponse,
        routing::post,
    };
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, sync::Mutex};

    use crate::{MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner};

    const AS_TOKEN: &str = "as-token-for-tests";
    const HUMAN: &str = "@rainy:matrix.agent-room.localhost";

    #[derive(Default)]
    struct Uploaded {
        requests: Mutex<Vec<(String, Value)>>,
    }

    #[tokio::test]
    async fn 以应用服务的身份冒充本人上传新的签名公钥() {
        let (url, uploaded) = start(StatusCode::OK).await;
        provisioner(&url)
            .replace_cross_signing_keys(&user(HUMAN), &keys(HUMAN))
            .await
            .expect("应用服务可以代传");
        let requests = uploaded.requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, HUMAN, "带上要冒充的用户");
        assert_eq!(
            &requests[0].1,
            &Value::Object(keys(HUMAN).as_json().clone())
        );
        assert!(requests[0].1.get("auth").is_none(), "不带交互认证");
    }

    #[tokio::test]
    async fn 别的服务器的用户不代传() {
        let (url, uploaded) = start(StatusCode::OK).await;
        let remote = "@rainy:remote.example";
        let failure = provisioner(&url)
            .replace_cross_signing_keys(&user(remote), &keys(remote))
            .await
            .expect_err("不得替远端用户上传");
        assert_eq!(failure.kind(), MatrixFailureKind::Forbidden);
        assert!(uploaded.requests.lock().await.is_empty());
    }

    #[tokio::test]
    async fn synapse_拒绝时报失败() {
        let (url, _) = start(StatusCode::FORBIDDEN).await;
        let failure = provisioner(&url)
            .replace_cross_signing_keys(&user(HUMAN), &keys(HUMAN))
            .await
            .expect_err("Synapse 拒绝冒充时报失败");
        assert_eq!(failure.kind(), MatrixFailureKind::Forbidden);
    }

    fn user(value: &str) -> MatrixUserId {
        MatrixUserId::new(value).expect("用户有效")
    }

    fn keys(owner: &str) -> MatrixCrossSigningKeys {
        MatrixCrossSigningKeys::new(
            json!({
                "master_key": { "user_id": owner, "usage": ["master"], "keys": { "ed25519:m": "m" } },
                "self_signing_key": { "user_id": owner, "usage": ["self_signing"], "keys": { "ed25519:s": "s" } },
            }),
            &user(owner),
        )
        .expect("公钥有效")
    }

    fn provisioner(url: &str) -> MatrixApplicationServiceProvisioner {
        MatrixApplicationServiceProvisioner::new(
            MatrixApplicationServiceConfiguration::new(
                url,
                "matrix.agent-room.localhost".to_owned(),
                SecretValue::new(AS_TOKEN.to_owned()).expect("令牌有效"),
                Duration::from_secs(5),
            )
            .expect("配置有效"),
        )
        .expect("客户端有效")
    }

    async fn start(status: StatusCode) -> (String, Arc<Uploaded>) {
        let uploaded = Arc::new(Uploaded::default());
        let app = Router::new()
            .route(
                "/_matrix/client/v3/keys/device_signing/upload",
                post(
                    move |State(state): State<Arc<Uploaded>>,
                          Query(query): Query<std::collections::BTreeMap<String, String>>,
                          headers: HeaderMap,
                          Json(body): Json<Value>| async move {
                        assert_eq!(
                            headers
                                .get(AUTHORIZATION)
                                .and_then(|value| value.to_str().ok()),
                            Some(format!("Bearer {AS_TOKEN}").as_str())
                        );
                        if status != StatusCode::OK {
                            return (
                                status,
                                Json(json!({ "errcode": "M_FORBIDDEN", "error": "no" })),
                            )
                                .into_response();
                        }
                        state
                            .requests
                            .lock()
                            .await
                            .push((query.get("user_id").cloned().unwrap_or_default(), body));
                        (StatusCode::OK, Json(json!({}))).into_response()
                    },
                ),
            )
            .with_state(uploaded.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("本地测试端口");
        let address = listener.local_addr().expect("本地地址");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("测试服务器运行");
        });
        (format!("http://{address}"), uploaded)
    }
}
