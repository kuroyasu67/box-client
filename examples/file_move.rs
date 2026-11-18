//! ファイルを移動するサンプル
mod common;

use box_client::{FileId, FolderId, MoveDestination, UpdateItemOptions, UserId};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=file_move -- <file-id> (<parent-folder-id> | --user <user-id>)";

/// ファイルを移動する。
///
/// 移動先には、親フォルダのID、または`--user`に続けてルートフォルダへ移動するユーザーのIDを指定する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 移動するファイルのIDと移動先を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_id, destination) = match args.as_slice() {
        [file_id, flag, user_id] if flag == "--user" => (
            FileId::new(file_id),
            MoveDestination::User(UserId::new(user_id)),
        ),
        [file_id, parent_id] => (
            FileId::new(file_id),
            MoveDestination::Folder(FolderId::new(parent_id)),
        ),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    // ファイルの最新のETagを取得し、取得後にファイルが変更されていた場合は移動しない。
    let file = client.file_information(&file_id).await?;
    let mut options = UpdateItemOptions::default();
    if let Some(etag) = file.etag.as_deref() {
        options = options.with_etag(etag);
    }

    let file = client.move_file(&file_id, &destination, &options).await?;
    tracing::info!("moved file: {file:?}");

    Ok(())
}
