mod common;

use box_client::{BoxItem, api::trash::ListTrashedItemsOptions};

/// ゴミ箱内の最初のファイルの情報を表示する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = common::setup().await?;

    let items = client
        .list_all_trashed_items(&ListTrashedItemsOptions::default())
        .await?;
    let file_item = items.iter().find_map(|item| match item {
        BoxItem::File(file) => Some(file),
        _ => None,
    });
    if let Some(file_item) = file_item {
        let file = client.trashed_file_information(&file_item.id).await?;
        tracing::info!("trashed file information: {file:?}");
    } else {
        tracing::info!("there is no file in the trash");
    }

    Ok(())
}
