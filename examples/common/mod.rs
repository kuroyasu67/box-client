use std::time::Duration;

use box_client::{
    BoxClient, BoxClientTimeout, BoxConfig, BoxItem, RetryPolicy,
    api::trash::ListTrashedItemsOptions,
};

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
        10,
        Duration::from_millis(200),
        Duration::from_secs(60),
        Duration::from_secs(1),
    )?;

    let client = BoxClient::builder(BoxConfig::from_file(BOX_CONFIG_FILE)?)
        .timeout(timeout)
        .retry(retry)
        .build()
        .await?;

    Ok(client)
}

/// ゴミ箱内のすべての項目を表示する。
#[allow(
    dead_code,
    reason = "サンプルごとに`common`モジュールをコンパイルするため、この関数を呼び出さないサンプルでは未使用になる"
)]
pub async fn list_trashed_items(client: &BoxClient) -> anyhow::Result<()> {
    let items = client
        .list_all_trashed_items(&ListTrashedItemsOptions::default())
        .await?;
    for item in &items {
        match item {
            BoxItem::File(file) => tracing::info!("file: id={}, name={}", file.id, file.name),
            BoxItem::Folder(folder) => {
                tracing::info!("folder: id={}, name={}", folder.id, folder.name)
            }
            BoxItem::WebLink(link) => {
                tracing::info!("web_link: id={}, name={}", link.id, link.name)
            }
            // `BoxItem`は`#[non_exhaustive]`であるため、`BoxItem::Unknown`と将来追加される種類をまとめて扱う。
            _ => tracing::info!("unknown item"),
        }
    }

    Ok(())
}
