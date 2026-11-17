mod common;

use box_client::{
    BoxItem,
    api::trash::{ListTrashedItemsOptions, RestoreFolderOptions},
};

/// ゴミ箱内の最初のフォルダを復元する。
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
    let Some(folder_item) = folder_item else {
        tracing::info!("there is no folder in the trash");
        return Ok(());
    };

    let folder = client
        .restore_folder(&folder_item.id, &RestoreFolderOptions::default())
        .await?;
    tracing::info!("restored folder: {folder:?}");

    common::list_trashed_items(&client).await
}
