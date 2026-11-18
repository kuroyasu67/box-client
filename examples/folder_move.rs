//! フォルダを移動するサンプル
mod common;

use box_client::{FolderId, UpdateFolderOptions};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=folder_move -- <folder-id> <parent-folder-id>";

/// フォルダを移動する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 移動するフォルダのIDと移動先の親フォルダのIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (folder_id, parent_id) = match args.as_slice() {
        [folder_id, parent_id] => (FolderId::new(folder_id), FolderId::new(parent_id)),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    // フォルダの最新のETagを取得し、取得後にフォルダが変更されていた場合は移動しない。
    let folder = client.folder_information(&folder_id).await?;
    let mut options = UpdateFolderOptions::default();
    if let Some(etag) = folder.etag.as_deref() {
        options = options.with_etag(etag);
    }

    let folder = client.move_folder(&folder_id, &parent_id, &options).await?;
    tracing::info!("moved folder: {folder:?}");

    Ok(())
}
