mod common;

use box_client::{DeleteFolderOptions, FolderId, RestoreItemOptions};

/// 使用方法
const USAGE: &str =
    "usage: RUST_LOG=info cargo run --example=folder_operations -- <folder-id> [--skip-to-restore]";

/// フォルダを操作する。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=folder_operations -- <folder-id>
/// ```
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 先頭の要素はプログラム名であるため読み飛ばす
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (folder_id, skip_to_restore) = match args.as_slice() {
        [folder_id] => (FolderId::new(folder_id.as_str()), false),
        [folder_id, flag] if flag == "--skip-to-restore" => {
            (FolderId::new(folder_id.as_str()), true)
        }
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    tracing::info!("folder id: {folder_id}");

    if !skip_to_restore {
        // フォルダの情報を取得
        match client.folder_information(&folder_id).await {
            Ok(folder) => tracing::info!("folder: {folder:?}"),
            Err(e) => anyhow::bail!(e),
        }

        // フォルダを削除
        let delete_options = DeleteFolderOptions {
            recursive: true,
            etag: None,
        };
        match client.delete_folder(&folder_id, &delete_options).await {
            Ok(()) => tracing::info!("folder was deleted successfully"),
            Err(e) => anyhow::bail!(e),
        }

        // 待機
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        // ゴミ箱にある削除したフォルダの情報を取得
        match client.trashed_folder_information(&folder_id).await {
            Ok(folder) => {
                tracing::info!("trashed folder information was fetched successfully: {folder:?}");
            }
            Err(e) => anyhow::bail!(e),
        }
    }

    // フォルダを復元
    let restore_options = RestoreItemOptions::default();
    match client.restore_folder(&folder_id, &restore_options).await {
        Ok(_) => tracing::info!("folder was restored successfully"),
        Err(e) => anyhow::bail!(e),
    }

    // 待機
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // 復元したフォルダの情報を取得
    match client.folder_information(&folder_id).await {
        Ok(folder) => tracing::info!("folder: {folder:?}"),
        Err(e) => anyhow::bail!(e),
    }

    Ok(())
}
