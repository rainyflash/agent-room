//! 改好密码以后替人注销别的聊天设备（`specs/change-password/design.md`）：以应用服务的身份冒充本人
//! 列出 Synapse 上他的设备，除了要留的那台一次注销。应用服务调 `delete_devices` 不要交互认证
//! （Synapse 1.159 的 `DeleteDevicesRestServlet`，MSC4190），冒充本人靠应用服务注册里覆盖本地用户的
//! 非独占命名空间（`tools/prodops/render.py`，ADR 0011）。

use agent_room_application::ports::{
    MatrixDeviceId, MatrixHumanDeviceSignOut, MatrixOperation, MatrixResult, MatrixUserId,
    PortFuture,
};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::{
    MatrixApplicationServiceProvisioner, decode_json, decode_matrix_error, map_matrix_error,
    map_transport_error, read_limited_body,
};

#[derive(Deserialize)]
struct DeviceList {
    devices: Vec<ListedDevice>,
}

#[derive(Deserialize)]
struct ListedDevice {
    device_id: String,
}

#[derive(Serialize)]
struct DeleteDevices<'a> {
    devices: &'a [String],
}

impl MatrixApplicationServiceProvisioner {
    async fn sign_out_other_human_devices(
        &self,
        user_id: &MatrixUserId,
        keep: Option<&MatrixDeviceId>,
    ) -> MatrixResult<usize> {
        let operation = MatrixOperation::SignOutHumanDevices;
        self.human_localpart(user_id, operation)?;
        let listed: DeviceList = self
            .as_user(Method::GET, "_matrix/client/v3/devices", user_id, None)
            .await?;
        let others: Vec<String> = listed
            .devices
            .into_iter()
            .map(|device| device.device_id)
            .filter(|device| keep.is_none_or(|keep| keep.as_str() != device))
            .collect();
        if others.is_empty() {
            return Ok(0);
        }
        let body = serde_json::to_value(DeleteDevices { devices: &others })
            .map_err(|_| crate::invalid_response(operation))?;
        let _: serde_json::Value = self
            .as_user(
                Method::POST,
                "_matrix/client/v3/delete_devices",
                user_id,
                Some(&body),
            )
            .await?;
        Ok(others.len())
    }

    /// 以应用服务的身份冒充 `user_id` 发一个请求，回答按 `T` 读。
    async fn as_user<T: for<'de> Deserialize<'de>>(
        &self,
        method: Method,
        path: &str,
        user_id: &MatrixUserId,
        body: Option<&serde_json::Value>,
    ) -> MatrixResult<T> {
        let operation = MatrixOperation::SignOutHumanDevices;
        let mut endpoint = self.endpoint(path, operation)?;
        endpoint
            .query_pairs_mut()
            .append_pair("user_id", user_id.as_str());
        let request = self
            .client
            .request(method, endpoint)
            .bearer_auth(self.access_token.expose());
        let request = match body {
            Some(body) => request.json(body),
            None => request,
        };
        let response = request
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if !status.is_success() {
            let error = decode_matrix_error(&body, operation)?;
            return Err(map_matrix_error(operation, status, &error));
        }
        decode_json(&body, operation)
    }
}

impl MatrixHumanDeviceSignOut for MatrixApplicationServiceProvisioner {
    fn sign_out_other_devices<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
        keep: Option<&'a MatrixDeviceId>,
    ) -> PortFuture<'a, MatrixResult<usize>> {
        Box::pin(self.sign_out_other_human_devices(user_id, keep))
    }
}

#[cfg(test)]
#[path = "human_devices/tests.rs"]
mod tests;
