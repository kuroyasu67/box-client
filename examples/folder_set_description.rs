//! フォルダの説明を設定するサンプル
mod common;

use box_client::{FolderId, UpdateItemOptions};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=folder_set_description -- <folder-id> <description>";

/// フォルダの説明を設定する。
///
/// 説明を削除する場合は、`<description>`に空文字列（`""`）を指定する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 説明を設定するフォルダのIDと説明を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (folder_id, description) = match args.as_slice() {
        [folder_id, description] => (FolderId::new(folder_id), description.clone()),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    // フォルダの最新のETagを取得し、取得後にフォルダが変更されていた場合は説明を設定しない。
    let folder = client.folder_information(&folder_id).await?;
    let mut options = UpdateItemOptions::default();
    if let Some(etag) = folder.etag.as_deref() {
        options = options.with_etag(etag);
    }

    let folder = client
        .set_folder_description(&folder_id, &description, &options)
        .await?;
    tracing::info!("set folder description: {:?}", folder.description);

    Ok(())
}
