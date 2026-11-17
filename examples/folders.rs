mod common;

use box_client::{BoxItem, FolderId, api::folders::ListItemsOptions};

/// `9010_一時共有フォルダのフォルダID`
const TEMPORARY_SHARING_FOLDER_ID: &str = "180035284927";

/// ルートフォルダ内のすべての項目を表示する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = common::setup().await?;

    let folder_id = FolderId::new(TEMPORARY_SHARING_FOLDER_ID);

    let items = client
        .list_all_items_in_folder(&folder_id, &ListItemsOptions::default())
        .await?;
    for item in &items {
        match item {
            BoxItem::File(file) => tracing::info!("file: id={}, name={}", file.id, file.name),
            BoxItem::Folder(folder) => {
                tracing::info!("folder: id={}, name={}", folder.id.as_ref(), folder.name)
            }
            BoxItem::WebLink(link) => {
                tracing::info!("web_link: id={}, name={}", link.id, link.name)
            }
            BoxItem::Unknown => tracing::info!("unknown item"),
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
    }

    // ファイルの情報を取得
    let file_item = items.iter().find_map(|item| match item {
        BoxItem::File(file) => Some(file),
        _ => None,
    });
    if let Some(file_item) = file_item {
        let file = client.file_information(&file_item.id).await?;
        tracing::info!("file information: {file:?}");
    }

    Ok(())
}
