//! フォルダ名を変更するサンプル
mod common;

use box_client::{FolderId, RenameFolderOptions};

/// 使用方法
const USAGE: &str =
    "usage: RUST_LOG=info cargo run --quiet --example=folder_rename -- <folder-id> <new-name>";

/// フォルダ名を変更する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 名前を変更するフォルダのIDと新しいフォルダ名を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (folder_id, name) = match args.as_slice() {
        [folder_id, name] => (FolderId::new(folder_id), name.clone()),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    // フォルダの最新のETagを取得し、取得後にフォルダが変更されていた場合は名前を変更しない。
    let folder = client.folder_information(&folder_id).await?;
    let mut options = RenameFolderOptions::default();
    if let Some(etag) = folder.etag.as_deref() {
        options = options.with_etag(etag);
    }

    let folder = client.rename_folder(&folder_id, &name, &options).await?;
    tracing::info!("renamed folder: {folder:?}");

    Ok(())
}
