//! サンプルで共有する処理
//!
//! サンプルごとに`common`モジュールをコンパイルするため、サンプルが使用しない項目は未使用になる。
//! そのため、モジュール全体で未使用の警告を抑制する。
#![allow(
    dead_code,
    reason = "サンプルごとに`common`モジュールをコンパイルするため、サンプルが使用しない項目は未使用になる"
)]

use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use box_client::{
    BoxClient, BoxConfig, BoxItem, ListTrashedItemsOptions, MIN_CHUNKED_UPLOAD_THRESHOLD,
    RetryPolicy, TimeoutConfig, UpdateItemOptions,
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

    // 分割アップロードに切り替えるファイルサイズを最小値の20MiBとし、1回のリクエストで送信するデータを20MiB以下にする。
    // 分割アップロードでは、部分（BOXが決める。例えば8MiB）を既定の並列数の4つまで同時に送信するため、
    // 通信帯域を分け合う部分のデータは最大で32MiB程度になる。
    // 上りの通信速度が毎秒2メガビット程度でも、どちらの送信も終えられるように、リクエストのタイムアウトを5分とする。
    // ダウンロードにはリクエストのタイムアウトを適用しないため、データを30秒受信しない場合にタイムアウトさせる。
    let timeout = TimeoutConfig::default()
        .with_request_timeout(Duration::from_secs(300))
        .with_connect_timeout(Duration::from_secs(5))
        .with_download_read_timeout(Duration::from_secs(30));
    let retry = RetryPolicy::default()
        .with_max_retries(10)
        .with_initial_delay(Duration::from_millis(200))
        .with_max_delay(Duration::from_secs(60))
        .with_max_jitter(Duration::from_secs(1));

    let config_file = std::env::var_os(BOX_CONFIG_FILE_ENV)
        .map_or_else(|| PathBuf::from(DEFAULT_BOX_CONFIG_FILE), PathBuf::from);
    let client = BoxClient::builder(BoxConfig::from_file(config_file)?)
        .timeout(timeout)
        .retry(retry)
        .chunked_upload_threshold(MIN_CHUNKED_UPLOAD_THRESHOLD)
        .build()
        .await?;

    Ok(client)
}

/// 削除または復元した項目の状態をBOXが反映するまで待機する時間
pub const WAIT_FOR_TRASH: Duration = Duration::from_secs(3);

/// サンプルが作成する項目の名前に付ける接頭辞を返す。
///
/// 接頭辞は、サンプルの実行ごとに異なる現在のUNIX時刻（秒）を含む。
/// したがって、サンプルを繰り返し実行しても、作成する項目の名前が既存の項目と衝突しない。
pub fn name_prefix() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    format!("box-client-example-{seconds}")
}

/// `ETag`を指定した場合は、`ETag`を設定した項目の更新オプションを返す。
///
/// 項目を更新するメソッドは、項目が`ETag`の取得後に変更されていると、項目を更新せずにエラーを返す。
pub fn update_options(etag: Option<&str>) -> UpdateItemOptions<'_> {
    let options = UpdateItemOptions::default();
    match etag {
        Some(etag) => options.with_etag(etag),
        None => options,
    }
}

/// ゴミ箱内のすべての項目を表示する。
pub async fn list_trashed_items(client: &BoxClient) -> anyhow::Result<()> {
    let items = client
        .list_all_trashed_items(&ListTrashedItemsOptions::default())
        .await?;
    log_items(&items);

    Ok(())
}

/// 項目の種類、IDおよび名前を表示する。
pub fn log_items(items: &[BoxItem]) {
    for item in items {
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
}
