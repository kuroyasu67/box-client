//! フォルダを移動するサンプル
mod common;

use box_client::{FolderId, MoveDestination, UpdateFolderOptions, UserId};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=folder_move -- <folder-id> (<parent-folder-id> | --user <user-id>)";

/// フォルダを移動する。
///
/// 移動先には、親フォルダのID、または`--user`に続けてルートフォルダへ移動するユーザーのIDを指定する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 移動するフォルダのIDと移動先を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (folder_id, destination) = match args.as_slice() {
        [folder_id, flag, user_id] if flag == "--user" => (
            FolderId::new(folder_id),
            MoveDestination::User(UserId::new(user_id)),
        ),
        [folder_id, parent_id] => (
            FolderId::new(folder_id),
            MoveDestination::Folder(FolderId::new(parent_id)),
        ),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    // フォルダの最新のETagを取得し、取得後にフォルダが変更されていた場合は移動しない。
    let folder = client.folder_information(&folder_id).await?;
    let mut options = UpdateFolderOptions::default();
    if let Some(etag) = folder.etag.as_deref() {
        options = options.with_etag(etag);
    }

    let folder = client
        .move_folder(&folder_id, &destination, &options)
        .await?;
    tracing::info!("moved folder: {folder:?}");

    Ok(())
}
