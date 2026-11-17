use std::time::Duration;

use box_client::{BoxClient, BoxClientTimeout, BoxConfig, RetryPolicy};

/// Box設定ファイルのパス
// cspell:disable-next-line
const BOX_CONFIG_FILE: &str = "resources/secrets/26695994_fhugry6j_config.json";

/// `RUST_LOG`に従ってログを出力するサブスクライバーを登録し、Box設定ファイルで認証したクライアントを構築する。
pub async fn setup() -> anyhow::Result<BoxClient> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_ansi(false)
        .init();

    let timeout = BoxClientTimeout {
        request_timeout: Duration::from_secs(10),
        connect_timeout: Duration::from_secs(5),
    };
    let retry = RetryPolicy::new(
        5,
        Duration::from_millis(200),
        Duration::from_secs(10),
        Duration::from_secs(1),
    )?;

    let client = BoxClient::builder(BoxConfig::from_file(BOX_CONFIG_FILE)?)
        .timeout(timeout)
        .retry(retry)
        .build()
        .await?;

    Ok(client)
}
