use std::{env, fs, process::ExitCode, time::Duration};

use agent_room_application::ports::SecretValue;
use agent_room_content_adapter::{S3ContentStoreConfig, S3PrivateContentObjectStore};

const RETRY_BUDGET: Duration = Duration::from_mins(1);
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let configuration = S3ContentStoreConfig::new(
        required_text("AGENT_ROOM_CONTENT_S3_ENDPOINT")?,
        required_text("AGENT_ROOM_CONTENT_S3_BUCKET")?,
        required_text("AGENT_ROOM_CONTENT_S3_REGION")?,
        required_secret("AGENT_ROOM_CONTENT_S3_ACCESS_KEY")?,
        required_secret("AGENT_ROOM_CONTENT_S3_SECRET_KEY")?,
        Duration::from_secs(5),
    )
    .map_err(|error| format!("对象存储初始化配置无效：{error}"))?;
    let create_if_missing = boolean("AGENT_ROOM_CONTENT_S3_CREATE_BUCKET")?;
    let store = S3PrivateContentObjectStore::new(&configuration);
    let deadline = tokio::time::Instant::now() + RETRY_BUDGET;

    loop {
        match store.ensure_bucket(create_if_missing).await {
            Ok(()) => return Ok(()),
            Err(error) if tokio::time::Instant::now() < deadline => {
                eprintln!("对象存储尚未就绪，稍后重试：{error}");
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(error) => return Err(format!("对象存储初始化失败：{error}")),
        }
    }
}

fn required_text(name: &'static str) -> Result<String, String> {
    let value = env::var(name).map_err(|_| format!("缺少必需配置：{name}"))?;
    if value.trim().is_empty() || value.len() > 1_024 || value.chars().any(char::is_control) {
        return Err(format!("配置无效：{name}"));
    }
    Ok(value.trim().to_owned())
}

fn required_secret(name: &'static str) -> Result<SecretValue, String> {
    let direct = env::var(name).ok();
    let file_name = format!("{name}_FILE");
    let file = env::var(&file_name).ok();
    let value = match (direct, file) {
        (Some(_), Some(_)) => return Err(format!("不得同时设置 {name} 与 {file_name}")),
        (Some(value), None) => value,
        (None, Some(path)) => fs::read_to_string(path)
            .map_err(|_| format!("无法读取 {name} 指向的 Secret 文件"))?
            .trim_end_matches(['\r', '\n'])
            .to_owned(),
        (None, None) => return Err(format!("缺少必需 Secret：{name}")),
    };
    SecretValue::new(value).map_err(|_| format!("Secret 无效：{name}"))
}

fn boolean(name: &'static str) -> Result<bool, String> {
    match required_text(name)?.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{name} 必须是 true 或 false")),
    }
}
