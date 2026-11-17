mod common;

use box_client::{BoxItem, api::trash::ListTrashedItemsOptions};

/// ゴミ箱内の最初のフォルダの情報を表示する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = common::setup().await?;

    let items = client
        .list_all_trashed_items(&ListTrashedItemsOptions::default())
        .await?;
    let folder_item = items.iter().find_map(|item| match item {
        BoxItem::Folder(folder) => Some(folder),
        _ => None,
    });
    if let Some(folder_item) = folder_item {
        let folder = client.trashed_folder_information(&folder_item.id).await?;
        tracing::info!("trashed folder information: {folder:?}");
    } else {
        tracing::info!("there is no folder in the trash");
    }

    Ok(())
}
