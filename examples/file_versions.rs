//! ファイルの過去のバージョンをリストするサンプル
mod common;

use box_client::{FileId, ListFileVersionsOptions};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=file_versions -- <file-id>";

/// ファイルの過去のすべてのバージョンをリストする。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ファイルのIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file_id = match args.as_slice() {
        [file_id] => FileId::new(file_id),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    let versions = client
        .list_all_file_versions(&file_id, &ListFileVersionsOptions::default())
        .await?;
    tracing::info!("number of versions: {}", versions.len());
    for version in versions {
        tracing::info!(
            "version: id={}, version_number={:?}, size={:?}, created_at={:?}",
            version.id,
            version.version_number,
            version.size,
            version.created_at
        );
    }

    Ok(())
}
