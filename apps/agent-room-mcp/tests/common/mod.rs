use serde_json::Value;

/// Read a completed MCP SSE response, ignoring transport keepalives.
pub async fn rpc_result(response: reqwest::Response) -> Value {
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let text = response.text().await.unwrap();
    let frames: Vec<Value> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| serde_json::from_str(data.trim()).unwrap())
        .collect();
    assert_eq!(frames.len(), 1, "expected one terminal response: {text}");
    frames.into_iter().next().unwrap()
}
