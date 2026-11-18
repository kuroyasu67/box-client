//! フォルダを操作するメソッドを順に呼び出すサンプル
//!
//! サンプルは、次の順にフォルダを操作する。
//!
//! 1. フォルダを作成し、情報を取得する。
//! 2. 親フォルダ内のすべての項目を表示する。
//! 3. フォルダ名を変更し、説明を設定する。
//! 4. タグ、共有リンクなどの複数の属性をまとめて更新してから、共有リンクを削除する。
//! 5. フォルダをコピーする。
//! 6. 移動先の親フォルダを指定した場合は、フォルダを移動する。
//! 7. フォルダを削除し、ゴミ箱内のフォルダの情報を取得してから、フォルダを復元する。
//!
//! サンプルが作成およびコピーしたフォルダは、サンプルの終了後もBOXに残る。
mod common;

use box_client::{
    BoxClient, CopyFolderOptions, DeleteFolderOptions, Folder, FolderId, FolderUpdate,
    ListItemsOptions, MoveDestination, RestoreItemOptions, SharedLinkAccess, SharedLinkSettings,
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
    common::log_items(&items);

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

    let folder = update_folder_attributes(&client, &folder_id, folder.etag.as_deref()).await?;

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

/// フォルダの複数の属性を1回のリクエストでまとめて更新してから、共有リンクなどを削除する。
async fn update_folder_attributes(
    client: &BoxClient,
    folder_id: &FolderId,
    etag: Option<&str>,
) -> anyhow::Result<Folder> {
    // フォルダの複数の属性を1回のリクエストで更新
    // タグと共有リンクを設定し、所有者以外のユーザーによるコラボレーターの招待を禁止する。
    // 指定しない属性は送信せず、BOXは変更しない。
    let tags = ["box-client", "example"];
    let update = FolderUpdate::default()
        .with_tags(&tags)
        .with_shared_link(
            SharedLinkSettings::default().with_access(SharedLinkAccess::Collaborators),
        )
        .with_can_non_owners_invite(false);
    let folder = client
        .update_folder(folder_id, &update, &common::update_options(etag))
        .await?;
    tracing::info!("updated folder: etag={:?}", folder.etag);

    // 共有リンクを削除
    // 削除する属性には、BOXにnullを送信する。
    let folder = client
        .update_folder(
            folder_id,
            &FolderUpdate::default().with_shared_link_removed(),
            &common::update_options(folder.etag.as_deref()),
        )
        .await?;
    tracing::info!("removed folder shared link: etag={:?}", folder.etag);

    Ok(folder)
}
