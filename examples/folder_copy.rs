//! フォルダをコピーするサンプル
mod common;

use box_client::{CopyFolderOptions, FolderId};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=folder_copy -- <folder-id> <parent-folder-id> [<new-name>]";

/// フォルダをコピーする。
///
/// `<new-name>`を省略した場合、BOXはコピー元のフォルダと同じ名前を付ける。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // コピーするフォルダのID、コピー先の親フォルダのID、およびコピーしたフォルダの名前を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (folder_id, parent_id, name) = match args.as_slice() {
        [folder_id, parent_id] => (FolderId::new(folder_id), FolderId::new(parent_id), None),
        [folder_id, parent_id, name] => (
            FolderId::new(folder_id),
            FolderId::new(parent_id),
            Some(name.as_str()),
        ),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    let mut options = CopyFolderOptions::default();
    if let Some(name) = name {
        options = options.with_name(name);
    }

    let folder = client.copy_folder(&folder_id, &parent_id, &options).await?;
    tracing::info!("copied folder: {folder:?}");

    Ok(())
}
