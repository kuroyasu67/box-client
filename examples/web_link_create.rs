//! Webリンクを作成するサンプル
mod common;

use box_client::{CreateWebLinkOptions, FolderId};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=web_link_create -- <url> <parent-folder-id> [<name>]";

/// Webリンクを作成する。
///
/// `<name>`を省略した場合、BOXはURLをWebリンク名にする。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // URL、親フォルダのID、およびWebリンク名を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (url, parent_id, name) = match args.as_slice() {
        [url, parent_id] => (url, FolderId::new(parent_id), None),
        [url, parent_id, name] => (url, FolderId::new(parent_id), Some(name.as_str())),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    let mut options = CreateWebLinkOptions::default();
    if let Some(name) = name {
        options = options.with_name(name);
    }

    let web_link = client.create_web_link(url, &parent_id, &options).await?;
    tracing::info!("created web link: {web_link:?}");

    Ok(())
}
