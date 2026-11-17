mod common;

use box_client::{FileId, api::trash::RestoreItemOptions};

/// 使用方法
const USAGE: &str =
    "usage: RUST_LOG=info cargo run --example=file_operations -- <file-id> [--skip-to-restore]";

/// ファイルを操作する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 先頭の要素はプログラム名であるため読み飛ばす
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_id, skip_to_restore) = match args.as_slice() {
        [file_id] => (FileId::new(file_id.as_str()), false),
        [file_id, flag] if flag == "--skip-to-restore" => (FileId::new(file_id.as_str()), true),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    tracing::info!("file id: {file_id}");

    if !skip_to_restore {
        // ファイルの情報を取得
        match client.file_information(&file_id).await {
            Ok(file) => tracing::info!("file: {file:?}"),
            Err(e) => anyhow::bail!(e),
        }

        // ファイルを削除
        match client.delete_file(&file_id, None).await {
            Ok(_) => tracing::info!("file was deleted successfully"),
            Err(e) => anyhow::bail!(e),
        }

        // 待機
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        // ゴミ箱にある削除したファイルの情報を取得
        match client.trashed_file_information(&file_id).await {
            Ok(file) => {
                tracing::info!("trashed file information was fetched successfully: {file:?}")
            }
            Err(e) => anyhow::bail!(e),
        }
    }

    // ファイルを復元
    let restore_options = RestoreItemOptions::default();
    match client.restore_file(&file_id, &restore_options).await {
        Ok(_) => tracing::info!("file was restored successfully"),
        Err(e) => anyhow::bail!(e),
    }

    // 待機
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // 復元したファイルの情報を取得
    match client.file_information(&file_id).await {
        Ok(file) => tracing::info!("restored file: {file:?}"),
        Err(e) => anyhow::bail!(e),
    }

    Ok(())
}
