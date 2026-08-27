use serde_json::Value;

#[test]
fn 主窗口能力不包含文件_shell_或进程通配权限() {
    let capability: Value = serde_json::from_str(include_str!("../capabilities/main.json"))
        .expect("主窗口能力文件必须是 JSON");
    let permissions = capability["permissions"]
        .as_array()
        .expect("主窗口能力必须声明权限")
        .iter()
        .map(|entry| entry.as_str().expect("权限标识必须是字符串"))
        .collect::<Vec<_>>();

    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    assert!(permissions.iter().all(|permission| {
        !permission.starts_with("fs:")
            && !permission.starts_with("shell:")
            && !permission.starts_with("process:")
            && !permission.contains('*')
    }));
    assert_eq!(
        permissions
            .iter()
            .filter(|permission| permission.starts_with("allow-desktop-"))
            .count(),
        11
    );
}

#[test]
fn 桌面_csp_禁止远程代码与可嵌入内容() {
    let config: Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("Tauri 配置必须是 JSON");
    let security = &config["app"]["security"];
    let csp = security["csp"].as_str().expect("桌面必须声明 CSP");
    let script_tokens = directive_tokens(csp, "script-src");

    assert!(script_tokens.contains(&"'self'"));
    assert!(script_tokens.contains(&"'wasm-unsafe-eval'"));
    assert!(!script_tokens.contains(&"'unsafe-eval'"));
    assert!(!script_tokens.contains(&"'unsafe-inline'"));
    assert!(csp.contains("object-src 'none'"));
    assert!(csp.contains("frame-src 'none'"));
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(csp.contains("base-uri 'none'"));
    assert!(csp.contains("upgrade-insecure-requests"));
    assert_eq!(
        security["headers"]["Permissions-Policy"],
        "camera=(), microphone=(), geolocation=(), payment=(), usb=()"
    );
    assert_eq!(security["headers"]["X-Content-Type-Options"], "nosniff");
}

#[test]
fn windows_alpha_仅生成支持语义化预发布版本的_nsis_安装器() {
    let config: Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("Tauri 配置必须是 JSON");

    assert_eq!(config["bundle"]["targets"], serde_json::json!(["nsis"]));
    assert_eq!(
        config["bundle"]["windows"]["nsis"]["installMode"],
        "currentUser"
    );
}

#[test]
fn 签名发行配置同时启用更新归档与_updater_插件() {
    let config: Value = serde_json::from_str(include_str!("../tauri.release.conf.json"))
        .expect("Tauri 签名发行配置必须是 JSON");

    assert_eq!(config["bundle"]["createUpdaterArtifacts"], true);
    assert!(
        config["plugins"]["updater"]["pubkey"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
}

fn directive_tokens<'a>(csp: &'a str, name: &str) -> Vec<&'a str> {
    csp.split(';')
        .map(str::trim)
        .find_map(|directive| {
            let mut tokens = directive.split_whitespace();
            (tokens.next() == Some(name)).then(|| tokens.collect())
        })
        .unwrap_or_default()
}
