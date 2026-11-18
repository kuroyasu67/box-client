//! ファイルを操作するメソッドを順に呼び出すサンプル
//!
//! サンプルは、次の順にファイルを操作する。
//!
//! 1. ファイルをアップロードし、情報を取得する。
//! 2. ファイル名を変更し、説明を設定する。
//! 3. ファイルをコピーする。
//! 4. 移動先の親フォルダを指定した場合は、ファイルを移動する。
//! 5. ファイルを削除し、ゴミ箱内のファイルの情報を取得してから、ファイルを復元する。
//! 6. ファイルの内容を更新し、過去のバージョンの取得、昇格、削除および復元を実行する。
//!
//! BOXは、プレミアムアカウントのユーザーのファイルだけにバージョンを記録する。
//! したがって、プレミアムアカウントでない場合は、6.のバージョンの操作でエラーが発生する。
//! サンプルがアップロードおよびコピーしたファイルは、サンプルの終了後もBOXに残る。
mod common;

use std::path::Path;

use box_client::{
    BoxClient, CopyFileOptions, File, FolderId, ListFileVersionsOptions, MoveDestination,
    RestoreItemOptions, UpdateFileContentOptions, UploadFileOptions,
};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=file -- <file-path> <parent-folder-id> [<destination-folder-id>]";

/// ファイルを操作するメソッドを順に呼び出す。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=file -- <file-path> <parent-folder-id> [<destination-folder-id>]
/// ```
///
/// `<file-path>`のファイルを`<parent-folder-id>`のフォルダにアップロードする。
/// 50MiBを超えるファイルは、分割アップロードのAPIでアップロードする。
/// `<destination-folder-id>`を指定した場合は、アップロードしたファイルをそのフォルダに移動する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // アップロードするファイルのパス、アップロード先の親フォルダのID、および移動先の親フォルダのIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_path, parent_id, destination_id) = match args.as_slice() {
        [file_path, parent_id] => (Path::new(file_path), FolderId::new(parent_id), None),
        [file_path, parent_id, destination_id] => (
            Path::new(file_path),
            FolderId::new(parent_id),
            Some(FolderId::new(destination_id)),
        ),
        _ => anyhow::bail!(USAGE),
    };
    let file_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("the file path has no valid file name"))?;

    let client = common::setup().await?;
    let prefix = common::name_prefix();

    // ファイルの内容を送信する前に、BOXがファイルを受け付けるかを確認してから、ファイルをアップロード
    let name = format!("{prefix}-{file_name}");
    let upload_options = UploadFileOptions::default().with_name(&name);
    let upload_url = client
        .preflight_upload_file(file_path, &parent_id, &upload_options)
        .await?;
    tracing::info!("preflight check passed: {upload_url:?}");
    let file = client
        .upload_file(file_path, &parent_id, &upload_options)
        .await?;
    tracing::info!("uploaded file: {file:?}");
    let file_id = file.id;

    // ファイルの情報を取得
    let file = client.file_information(&file_id).await?;
    tracing::info!("file information: {file:?}");

    // ファイル名を変更
    // 直前に取得した`ETag`を指定し、取得後にファイルが変更されていた場合は名前を変更しない。
    let name = format!("{prefix}-renamed-{file_name}");
    let file = client
        .rename_file(
            &file_id,
            &name,
            &common::update_options(file.etag.as_deref()),
        )
        .await?;
    tracing::info!("renamed file: {}", file.name);

    // ファイルの説明を設定
    // 説明を削除する場合は、空文字列を指定する。
    let file = client
        .set_file_description(
            &file_id,
            "box-clientのサンプルが設定した説明",
            &common::update_options(file.etag.as_deref()),
        )
        .await?;
    tracing::info!("set file description: {:?}", file.description);

    // 同じ親フォルダにファイルをコピー
    // 名前を指定しない場合、BOXはコピー元のファイルと同じ名前を付ける。
    let name = format!("{prefix}-copied-{file_name}");
    let copied = client
        .copy_file(
            &file_id,
            &parent_id,
            &CopyFileOptions::default().with_name(&name),
        )
        .await?;
    tracing::info!("copied file: id={}, name={}", copied.id, copied.name);

    // ファイルを移動
    // `MoveDestination::User`を指定した場合は、ユーザーのルートフォルダへ移動する。
    if let Some(destination_id) = destination_id {
        let file = client.file_information(&file_id).await?;
        let file = client
            .move_file(
                &file_id,
                &MoveDestination::Folder(destination_id),
                &common::update_options(file.etag.as_deref()),
            )
            .await?;
        tracing::info!("moved file: {:?}", file.path_collection);
    }

    // ファイルを削除し、ゴミ箱にある削除したファイルの情報を取得
    client.delete_file(&file_id, None).await?;
    tracing::info!("file was deleted successfully");
    tokio::time::sleep(common::WAIT_FOR_TRASH).await;
    let file = client.trashed_file_information(&file_id).await?;
    tracing::info!(
        "trashed file: id={}, trashed_at={:?}",
        file.id,
        file.trashed_at
    );

    // ファイルを復元
    client
        .restore_file(&file_id, &RestoreItemOptions::default())
        .await?;
    tracing::info!("file was restored successfully");
    tokio::time::sleep(common::WAIT_FOR_TRASH).await;
    let file = client.file_information(&file_id).await?;
    tracing::info!("restored file: {file:?}");

    operate_versions(&client, &file, file_path).await
}

/// ファイルの内容を更新し、過去のバージョンの取得、昇格、削除および復元を順に実行する。
async fn operate_versions(client: &BoxClient, file: &File, file_path: &Path) -> anyhow::Result<()> {
    let file_id = &file.id;

    // ファイルの内容を更新し、新しいバージョンを作成
    // 更新する前に、BOXが新しいバージョンを受け付けるかを確認する。
    let mut update_options = UpdateFileContentOptions::default();
    if let Some(etag) = file.etag.as_deref() {
        update_options = update_options.with_etag(etag);
    }
    let upload_url = client
        .preflight_update_file_content(file_id, file_path, &update_options)
        .await?;
    tracing::info!("preflight check passed: {upload_url:?}");
    let file = client
        .update_file_content(file_id, file_path, &update_options)
        .await?;
    tracing::info!("updated file: {:?}", file.file_version);

    // ファイルの過去のすべてのバージョンをリスト
    let versions = client
        .list_all_file_versions(file_id, &ListFileVersionsOptions::default())
        .await?;
    for version in &versions {
        tracing::info!(
            "version: id={}, version_number={:?}, size={:?}, created_at={:?}",
            version.id,
            version.version_number,
            version.size,
            version.created_at
        );
    }
    let previous_version_id = versions
        .first()
        .map(|version| version.id.clone())
        .ok_or_else(|| anyhow::anyhow!("the file has no previous version"))?;

    // 過去のバージョンの情報を取得
    let version = client.file_version(file_id, &previous_version_id).await?;
    tracing::info!("file version: {version:?}");

    // 過去のバージョンを昇格し、現在のバージョンにする
    // BOXは、昇格したバージョンの複製を、異なるIDを持つ現在のバージョンとして作成する。
    let version = client
        .promote_file_version(file_id, &previous_version_id)
        .await?;
    tracing::info!("promoted file version: {version:?}");

    // 過去のバージョンを削除
    // 直前に取得した`ETag`を指定し、取得後にファイルが変更されていた場合は削除しない。
    // BOXは、削除したバージョンをゴミ箱に移動する。
    let file = client.file_information(file_id).await?;
    client
        .delete_file_version(file_id, &previous_version_id, file.etag.as_deref())
        .await?;
    tracing::info!("file version was deleted successfully");

    // 削除したバージョンを、ファイルのバージョン履歴に戻す
    let version = client
        .restore_file_version(file_id, &previous_version_id)
        .await?;
    tracing::info!("restored file version: {version:?}");

    Ok(())
}
