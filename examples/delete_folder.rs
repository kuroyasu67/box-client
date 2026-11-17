mod common;

use box_client::{FolderId, api::folders::DeleteFolderOptions};

const REMOVE_FOLDER_ID: &str = "438260515094";

/// フォルダを削除する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = common::setup().await?;

    let folder_id = FolderId(REMOVE_FOLDER_ID.to_string());
    let options = DeleteFolderOptions {
        recursive: true,
        etag: None,
    };
    match client.delete_folder(&folder_id, &options).await {
        Ok(_) => tracing::info!("delete folder successfully"),
        Err(e) => tracing::error!("{e:?}"),
    }

    common::list_trashed_items(&client).await
}
