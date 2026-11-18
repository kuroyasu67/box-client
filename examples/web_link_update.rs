//! Webリンクの名前、説明、URLまたは親フォルダを更新するサンプル
mod common;

use box_client::{FolderId, MoveDestination, UpdateItemOptions, WebLinkId};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=web_link_update -- <web-link-id> (--name <name> | --description <description> | --url <url> | --move-to <parent-folder-id>)";

/// Webリンクの名前、説明、URLまたは親フォルダを更新する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // WebリンクのID、更新する属性および値を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [web_link_id, attribute, value] = args.as_slice() else {
        anyhow::bail!(USAGE);
    };
    let web_link_id = WebLinkId::new(web_link_id);

    let client = common::setup().await?;

    // Webリンクの最新のETagを取得し、取得後にWebリンクが変更されていた場合は更新しない。
    let web_link = client.web_link_information(&web_link_id).await?;
    let mut options = UpdateItemOptions::default();
    if let Some(etag) = web_link.etag.as_deref() {
        options = options.with_etag(etag);
    }

    let web_link = match attribute.as_str() {
        "--name" => {
            client
                .rename_web_link(&web_link_id, value, &options)
                .await?
        }
        "--description" => {
            client
                .set_web_link_description(&web_link_id, value, &options)
                .await?
        }
        "--url" => {
            client
                .update_web_link_url(&web_link_id, value, &options)
                .await?
        }
        "--move-to" => {
            let destination = MoveDestination::Folder(FolderId::new(value));
            client
                .move_web_link(&web_link_id, &destination, &options)
                .await?
        }
        _ => anyhow::bail!(USAGE),
    };
    tracing::info!("updated web link: {web_link:?}");

    Ok(())
}
