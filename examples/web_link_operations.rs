mod common;

use box_client::{RestoreItemOptions, WebLinkId};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --example=web_link_operations -- <web-link-id> [--skip-to-restore]";

/// Webリンクを操作する。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=web_link_operations -- <web-link-id>
/// ```
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 先頭の要素はプログラム名であるため読み飛ばす
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (web_link_id, skip_to_restore) = match args.as_slice() {
        [web_link_id] => (WebLinkId::new(web_link_id.as_str()), false),
        [web_link_id, flag] if flag == "--skip-to-restore" => {
            (WebLinkId::new(web_link_id.as_str()), true)
        }
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    tracing::info!("web link id: {web_link_id}");

    if !skip_to_restore {
        // Webリンクの情報を取得
        match client.web_link_information(&web_link_id).await {
            Ok(web_link) => tracing::info!("web link: {web_link:?}"),
            Err(e) => anyhow::bail!(e),
        }

        // Webリンクを削除
        match client.delete_web_link(&web_link_id).await {
            Ok(()) => tracing::info!("web link was deleted successfully"),
            Err(e) => anyhow::bail!(e),
        }

        // 待機
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        // ゴミ箱にある削除したWebリンクの情報を取得
        match client.trashed_web_link_information(&web_link_id).await {
            Ok(web_link) => {
                tracing::info!(
                    "trashed web link information was fetched successfully: {web_link:?}"
                );
            }
            Err(e) => anyhow::bail!(e),
        }
    }

    // Webリンクを復元
    let restore_options = RestoreItemOptions::default();
    match client
        .restore_web_link(&web_link_id, &restore_options)
        .await
    {
        Ok(_) => tracing::info!("web link was restored successfully"),
        Err(e) => anyhow::bail!(e),
    }

    // 待機
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // 復元したWebリンクの情報を取得
    match client.web_link_information(&web_link_id).await {
        Ok(web_link) => tracing::info!("restored web link: {web_link:?}"),
        Err(e) => anyhow::bail!(e),
    }

    Ok(())
}
