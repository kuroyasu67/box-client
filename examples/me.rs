use std::time::Duration;

use box_client::{BoxClient, BoxClientTimeout, BoxConfig, RetryPolicy};

/// Box設定ファイルのパス
// cspell:disable-next-line
const BOX_CONFIG_FILE: &str = "resources/secrets/26695994_fhugry6j_config.json";

/// Box設定ファイルを読み込んで認証し、サービスアカウントのユーザー情報を表示する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
    let user = client.me().await?;
    println!("id={}, name={}, login={}", user.id, user.name, user.login);

    Ok(())
}
