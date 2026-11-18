//! ファイルの過去のバージョンを昇格するサンプル
mod common;

use box_client::{FileId, FileVersionId};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=file_version_promote -- <file-id> <file-version-id>";

/// ファイルの過去のバージョンを昇格し、現在のバージョンにする。
///
/// 過去のバージョンのIDは、`file_versions`サンプルで確認できる。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ファイルのIDと昇格するバージョンのIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_id, file_version_id) = match args.as_slice() {
        [file_id, version_id] => (FileId::new(file_id), FileVersionId::new(version_id)),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    let version = client
        .promote_file_version(&file_id, &file_version_id)
        .await?;
    tracing::info!("promoted file version: {version:?}");

    Ok(())
}
