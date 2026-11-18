use std::{path::PathBuf, time::Duration};

use box_client::{
    BoxClient, BoxClientTimeout, BoxConfig, BoxItem, ListTrashedItemsOptions, RetryPolicy,
};

/// BOX設定ファイルのパスを指定する環境変数の名前
const BOX_CONFIG_FILE_ENV: &str = "BOX_CONFIG_FILE";

/// 環境変数でBOX設定ファイルのパスを指定しない場合に使用するパス
///
/// `resources/secrets`は`.gitignore`で除外しているため、BOX設定ファイルを誤ってコミットしない。
const DEFAULT_BOX_CONFIG_FILE: &str = "resources/secrets/config.json";

/// `RUST_LOG`に従ってログを出力するサブスクライバーを登録し、BOX設定ファイルで認証したクライアントを構築する。
///
/// BOX設定ファイルのパスは、環境変数`BOX_CONFIG_FILE`で指定する。
/// 指定しない場合は、`resources/secrets/config.json`を読み込む。
pub async fn setup() -> anyhow::Result<BoxClient> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_ansi(false)
        .init();

    // ファイルのアップロードでは、1回のリクエストで最大50MiBを送信する。
    // 上りの通信速度が毎秒2メガビット程度でも送信を終えられるように、リクエストのタイムアウトを5分とする。
    let timeout = BoxClientTimeout::default()
        .with_request_timeout(Duration::from_secs(300))
        .with_connect_timeout(Duration::from_secs(5));
    let retry = RetryPolicy::new(
        10,
        Duration::from_millis(200),
        Duration::from_secs(60),
        Duration::from_secs(1),
    )?;

    let config_file = std::env::var_os(BOX_CONFIG_FILE_ENV)
        .map_or_else(|| PathBuf::from(DEFAULT_BOX_CONFIG_FILE), PathBuf::from);
    let client = BoxClient::builder(BoxConfig::from_file(config_file)?)
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
                tracing::info!("folder: id={}, name={}", folder.id, folder.name);
            }
            BoxItem::WebLink(link) => {
                tracing::info!("web_link: id={}, name={}", link.id, link.name);
            }
            // `BoxItem`は`#[non_exhaustive]`であるため、`BoxItem::Unknown`と将来追加される種類をまとめて扱う。
            _ => tracing::info!("unknown item"),
        }
    }

    Ok(())
}
