//! Webリンクを操作するメソッドを順に呼び出すサンプル
//!
//! サンプルは、次の順にWebリンクを操作する。
//!
//! 1. Webリンクを作成し、情報を取得する。
//! 2. Webリンクの名前、説明およびURLを更新する。
//! 3. 名前、説明および共有リンクをまとめて更新してから、共有リンクを削除する。
//! 4. 移動先の親フォルダを指定した場合は、Webリンクを移動する。
//! 5. Webリンクを削除し、ゴミ箱内のWebリンクの情報を取得してから、Webリンクを復元する。
//!
//! サンプルが作成したWebリンクは、サンプルの終了後もBOXに残る。
mod common;

use box_client::{
    BoxClient, CreateWebLinkOptions, DeleteItemOptions, FolderId, MoveDestination,
    RestoreItemOptions, SharedLinkAccess, SharedLinkSettings, WebLink, WebLinkId, WebLinkUpdate,
};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=web_link -- <url> <new-url> <parent-folder-id> [<destination-folder-id>]";

/// Webリンクを操作するメソッドを順に呼び出す。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=web_link -- <url> <new-url> <parent-folder-id> [<destination-folder-id>]
/// ```
///
/// `<url>`を指すWebリンクを`<parent-folder-id>`のフォルダに作成し、後でURLを`<new-url>`に更新する。
/// `<destination-folder-id>`を指定した場合は、作成したWebリンクをそのフォルダに移動する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // URL、更新後のURL、親フォルダのID、および移動先の親フォルダのIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (url, new_url, parent_id, destination_id) = match args.as_slice() {
        [url, new_url, parent_id] => (url, new_url, FolderId::new(parent_id), None),
        [url, new_url, parent_id, destination_id] => (
            url,
            new_url,
            FolderId::new(parent_id),
            Some(FolderId::new(destination_id)),
        ),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;
    let prefix = common::name_prefix();

    // Webリンクを作成
    // 名前を指定しない場合、BOXはURLをWebリンク名にする。
    let web_link = client
        .create_web_link(
            url,
            &parent_id,
            &CreateWebLinkOptions::default().with_name(&prefix),
        )
        .await?;
    tracing::info!("created web link: {web_link:?}");
    let web_link_id = web_link.id;

    // Webリンクの情報を取得
    let web_link = client.web_link_information(&web_link_id).await?;
    tracing::info!("web link information: {web_link:?}");

    // Webリンクの名前を変更
    // 直前に取得した`ETag`を指定し、取得後にWebリンクが変更されていた場合は名前を変更しない。
    let name = format!("{prefix}-renamed");
    let web_link = client
        .rename_web_link(
            &web_link_id,
            &name,
            &common::update_options(web_link.etag.as_deref()),
        )
        .await?;
    tracing::info!("renamed web link: {}", web_link.name);

    // Webリンクの説明を設定
    // 説明を削除する場合は、空文字列を指定する。
    let web_link = client
        .set_web_link_description(
            &web_link_id,
            "box-clientのサンプルが設定した説明",
            &common::update_options(web_link.etag.as_deref()),
        )
        .await?;
    tracing::info!("set web link description: {:?}", web_link.description);

    // WebリンクのURLを更新
    let web_link = client
        .update_web_link_url(
            &web_link_id,
            new_url,
            &common::update_options(web_link.etag.as_deref()),
        )
        .await?;
    tracing::info!("updated web link url: {:?}", web_link.url);

    let web_link =
        update_web_link_attributes(&client, &web_link_id, web_link.etag.as_deref(), &prefix)
            .await?;

    // Webリンクを移動
    // `MoveDestination::User`を指定した場合は、ユーザーのルートフォルダへ移動する。
    if let Some(destination_id) = destination_id {
        let web_link = client
            .move_web_link(
                &web_link_id,
                &MoveDestination::Folder(destination_id),
                &common::update_options(web_link.etag.as_deref()),
            )
            .await?;
        tracing::info!("moved web link: {:?}", web_link.path_collection);
    }

    // Webリンクを削除し、ゴミ箱にある削除したWebリンクの情報を取得
    client
        .delete_web_link(&web_link_id, &DeleteItemOptions::default())
        .await?;
    tracing::info!("web link was deleted successfully");
    tokio::time::sleep(common::WAIT_FOR_TRASH).await;
    let web_link = client.trashed_web_link_information(&web_link_id).await?;
    tracing::info!(
        "trashed web link: id={}, trashed_at={:?}",
        web_link.id,
        web_link.trashed_at
    );

    // Webリンクを復元
    client
        .restore_web_link(&web_link_id, &RestoreItemOptions::default())
        .await?;
    tracing::info!("web link was restored successfully");
    tokio::time::sleep(common::WAIT_FOR_TRASH).await;
    let web_link = client.web_link_information(&web_link_id).await?;
    tracing::info!("restored web link: {web_link:?}");

    Ok(())
}

/// Webリンクの複数の属性を1回のリクエストでまとめて更新してから、共有リンクなどを削除する。
async fn update_web_link_attributes(
    client: &BoxClient,
    web_link_id: &WebLinkId,
    etag: Option<&str>,
    prefix: &str,
) -> anyhow::Result<WebLink> {
    // Webリンクの複数の属性を1回のリクエストで更新
    // 名前、説明および共有リンクを設定する。指定しない属性は送信せず、BOXは変更しない。
    let name = format!("{prefix}-updated");
    let update = WebLinkUpdate::default()
        .with_name(&name)
        .with_description("box-clientのサンプルがまとめて更新した説明")
        .with_shared_link(
            SharedLinkSettings::default().with_access(SharedLinkAccess::Collaborators),
        );
    let web_link = client
        .update_web_link(web_link_id, &update, &common::update_options(etag))
        .await?;
    tracing::info!(
        "updated web link: name={}, description={:?}",
        web_link.name,
        web_link.description
    );

    // 共有リンクを削除
    // 削除する属性には、BOXにnullを送信する。
    let web_link = client
        .update_web_link(
            web_link_id,
            &WebLinkUpdate::default().with_shared_link_removed(),
            &common::update_options(web_link.etag.as_deref()),
        )
        .await?;
    tracing::info!("removed web link shared link: etag={:?}", web_link.etag);

    Ok(web_link)
}
