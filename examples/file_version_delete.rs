//! ファイルのバージョンを削除するサンプル
mod common;

use box_client::{FileId, FileVersionId};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=file_version_delete -- <file-id> <file-version-id>";

/// ファイルのバージョンを削除する。
///
/// BOXは、削除したバージョンをゴミ箱に移動する。`file_version_restore`サンプルで復元できる。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ファイルのIDと削除するバージョンのIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_id, file_version_id) = match args.as_slice() {
        [file_id, version_id] => (FileId::new(file_id), FileVersionId::new(version_id)),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    // ファイルの最新のETagを取得し、取得後にファイルが変更されていた場合は削除しない。
    let file = client.file_information(&file_id).await?;
    client
        .delete_file_version(&file_id, &file_version_id, file.etag.as_deref())
        .await?;
    tracing::info!("file version was deleted successfully");

    Ok(())
}
