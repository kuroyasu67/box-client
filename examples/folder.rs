//! フォルダを操作するメソッドを順に呼び出すサンプル
//!
//! サンプルは、次の順にフォルダを操作する。
//!
//! 1. フォルダを作成し、情報を取得する。
//! 2. 親フォルダ内のすべての項目を表示する。
//! 3. フォルダ名を変更し、説明を設定する。
//! 4. フォルダをコピーする。
//! 5. 移動先の親フォルダを指定した場合は、フォルダを移動する。
//! 6. フォルダを削除し、ゴミ箱内のフォルダの情報を取得してから、フォルダを復元する。
//!
//! サンプルが作成およびコピーしたフォルダは、サンプルの終了後もBOXに残る。
mod common;

use box_client::{
    BoxItem, CopyFolderOptions, DeleteFolderOptions, FolderId, ListItemsOptions, MoveDestination,
    RestoreItemOptions,
};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=folder -- <parent-folder-id> [<destination-folder-id>]";

/// フォルダを操作するメソッドを順に呼び出す。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=folder -- <parent-folder-id> [<destination-folder-id>]
/// ```
///
/// `<parent-folder-id>`のフォルダにフォルダを作成する。
/// `<destination-folder-id>`を指定した場合は、作成したフォルダをそのフォルダに移動する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 作成するフォルダの親フォルダのIDと、移動先の親フォルダのIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (parent_id, destination_id) = match args.as_slice() {
        [parent_id] => (FolderId::new(parent_id), None),
        [parent_id, destination_id] => (
            FolderId::new(parent_id),
            Some(FolderId::new(destination_id)),
        ),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;
    let prefix = common::name_prefix();

    // フォルダを作成
    let folder = client.create_folder(&prefix, &parent_id).await?;
    tracing::info!("created folder: {folder:?}");
    let folder_id = folder.id;

    // フォルダの情報を取得
    let folder = client.folder_information(&folder_id).await?;
    tracing::info!("folder information: {folder:?}");

    // 親フォルダ内のすべての項目を表示
    let items = client
        .list_all_items_in_folder(&parent_id, &ListItemsOptions::default())
        .await?;
    for item in &items {
        match item {
            BoxItem::File(file) => tracing::info!("file: id={}, name={}", file.id, file.name),
            BoxItem::Folder(folder) => {
                tracing::info!("folder: id={}, name={}", folder.id, folder.name);
            }
            BoxItem::WebLink(link) => {
                tracing::info!("web_link: id={}, name={}", link.id, link.name);
            }
            // `BoxItem`は`#[non_exhaustive]`であるため、`BoxItem::Unknown`と将来追加される種類をまとめて扱う。
            _ => tracing::info!("unknown item"),
        }
    }

    // フォルダ名を変更
    // 直前に取得した`ETag`を指定し、取得後にフォルダが変更されていた場合は名前を変更しない。
    let name = format!("{prefix}-renamed");
    let folder = client
        .rename_folder(
            &folder_id,
            &name,
            &common::update_options(folder.etag.as_deref()),
        )
        .await?;
    tracing::info!("renamed folder: {}", folder.name);

    // フォルダの説明を設定
    // 説明を削除する場合は、空文字列を指定する。
    let folder = client
        .set_folder_description(
            &folder_id,
            "box-clientのサンプルが設定した説明",
            &common::update_options(folder.etag.as_deref()),
        )
        .await?;
    tracing::info!("set folder description: {:?}", folder.description);

    // 同じ親フォルダにフォルダをコピー
    // 名前を指定しない場合、BOXはコピー元のフォルダと同じ名前を付ける。
    let name = format!("{prefix}-copied");
    let copied = client
        .copy_folder(
            &folder_id,
            &parent_id,
            &CopyFolderOptions::default().with_name(&name),
        )
        .await?;
    tracing::info!("copied folder: id={}, name={}", copied.id, copied.name);

    // フォルダを移動
    // `MoveDestination::User`を指定した場合は、ユーザーのルートフォルダへ移動する。
    let folder = match destination_id {
        Some(destination_id) => {
            let folder = client
                .move_folder(
                    &folder_id,
                    &MoveDestination::Folder(destination_id),
                    &common::update_options(folder.etag.as_deref()),
                )
                .await?;
            tracing::info!("moved folder: {:?}", folder.path_collection);
            folder
        }
        None => folder,
    };

    // フォルダを削除し、ゴミ箱にある削除したフォルダの情報を取得
    // フォルダが空でない場合でも削除するように、再帰的な削除を指定する。
    let mut delete_options = DeleteFolderOptions::default().with_recursive(true);
    if let Some(etag) = folder.etag.as_deref() {
        delete_options = delete_options.with_etag(etag);
    }
    client.delete_folder(&folder_id, &delete_options).await?;
    tracing::info!("folder was deleted successfully");
    tokio::time::sleep(common::WAIT_FOR_TRASH).await;
    let folder = client.trashed_folder_information(&folder_id).await?;
    tracing::info!(
        "trashed folder: id={}, trashed_at={:?}",
        folder.id,
        folder.trashed_at
    );

    // フォルダを復元
    client
        .restore_folder(&folder_id, &RestoreItemOptions::default())
        .await?;
    tracing::info!("folder was restored successfully");
    tokio::time::sleep(common::WAIT_FOR_TRASH).await;
    let folder = client.folder_information(&folder_id).await?;
    tracing::info!("restored folder: {folder:?}");

    Ok(())
}
