//! フォルダを作成するサンプル
mod common;

use box_client::FolderId;

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=folder_creation -- <folder-name> <parent-folder-id>";

/// フォルダを作成する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 作成するフォルダ名と親フォルダのIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (name, parent_id) = match args.as_slice() {
        [name, id] => (name.clone(), FolderId::new(id)),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    let folder = client.create_folder(&name, &parent_id).await?;
    tracing::info!("created folder: {folder:?}");

    Ok(())
}
