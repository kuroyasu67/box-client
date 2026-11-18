//! 削除したファイルのバージョンを復元するサンプル
mod common;

use box_client::{FileId, FileVersionId};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=file_version_restore -- <file-id> <file-version-id>";

/// 削除してゴミ箱に移動したファイルのバージョンを、ファイルのバージョン履歴に戻す。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ファイルのIDと復元するバージョンのIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_id, file_version_id) = match args.as_slice() {
        [file_id, version_id] => (FileId::new(file_id), FileVersionId::new(version_id)),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    let version = client
        .restore_file_version(&file_id, &file_version_id)
        .await?;
    tracing::info!("restored file version: {version:?}");

    Ok(())
}
