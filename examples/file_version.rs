//! ファイルのバージョン情報を取得するサンプル
mod common;

use box_client::{FileId, FileVersionId};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=file_version -- <file-id> [<file-version-id>]";

/// ファイルのバージョン情報を取得する。
///
/// `<file-version-id>`を省略した場合は、ファイルの現在のバージョンの情報を取得する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ファイルのIDとバージョンのIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_id, file_version_id) = match args.as_slice() {
        [file_id] => (FileId::new(file_id), None),
        [file_id, version_id] => (FileId::new(file_id), Some(FileVersionId::new(version_id))),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    // バージョンのIDを指定しない場合は、ファイル情報から現在のバージョンのIDを取得
    let file_version_id = match file_version_id {
        Some(id) => id,
        None => client
            .file_information(&file_id)
            .await?
            .file_version
            .map(|version| version.id)
            .ok_or_else(|| anyhow::anyhow!("the file has no version"))?,
    };

    let version = client.file_version(&file_id, &file_version_id).await?;
    tracing::info!("file version: {version:?}");

    Ok(())
}
