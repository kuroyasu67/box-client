mod common;

use box_client::{BoxItem, FolderId, api::folders::ListItemsOptions};

/// フォルダ内のすべての項目を表示する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let folder_id = std::env::args().nth(1).map(FolderId::new).ok_or_else(|| {
        anyhow::anyhow!("usage: RUST_LOG=info cargo run --example=folder_list_items -- <folder-id>")
    })?;

    let client = common::setup().await?;

    let items = client
        .list_all_items_in_folder(&folder_id, &ListItemsOptions::default())
        .await?;
    for item in &items {
        match item {
            BoxItem::File(file) => tracing::info!("file: id={}, name={}", file.id, file.name),
            BoxItem::Folder(folder) => {
                tracing::info!("folder: id={}, name={}", folder.id.as_ref(), folder.name);
            }
            BoxItem::WebLink(link) => {
                tracing::info!("web_link: id={}, name={}", link.id, link.name);
            }
            // `BoxItem`は`#[non_exhaustive]`であるため、`BoxItem::Unknown`と将来追加される種類をまとめて扱う。
            _ => tracing::info!("unknown item"),
        }
    }

    // フォルダの情報を取得
    let folder_item = items.iter().find_map(|item| match item {
        BoxItem::Folder(folder) => Some(folder),
        _ => None,
    });
    if let Some(folder_item) = folder_item {
        let folder = client.folder_information(&folder_item.id).await?;
        tracing::info!("folder information: {folder:?}");
    } else {
        tracing::info!("there is no folder in the folder");
    }

    // ファイルの情報を取得
    let file_item = items.iter().find_map(|item| match item {
        BoxItem::File(file) => Some(file),
        _ => None,
    });
    if let Some(file_item) = file_item {
        let file = client.file_information(&file_item.id).await?;
        tracing::info!("file information: {file:?}");
    } else {
        tracing::info!("there is no file in the folder");
    }

    // Webリンクの情報を取得
    let link_item = items.iter().find_map(|item| match item {
        BoxItem::WebLink(link) => Some(link),
        _ => None,
    });
    if let Some(link_item) = link_item {
        let link = client.web_link_information(&link_item.id).await?;
        tracing::info!("web link information: {link:?}");
    } else {
        tracing::info!("there is no web link in the folder");
    }

    Ok(())
}
