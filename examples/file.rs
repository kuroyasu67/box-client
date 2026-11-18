//! ファイルを操作するメソッドを順に呼び出すサンプル
//!
//! サンプルは、次の順にファイルを操作する。
//!
//! 1. ファイルをアップロードし、情報を取得してから、ダウンロードする。
//! 2. ファイル名を変更し、説明を設定する。
//! 3. タグ、共有リンクおよびロックをまとめて更新してから、共有リンクとロックを削除する。
//! 4. ファイルをコピーする。
//! 5. 移動先の親フォルダを指定した場合は、ファイルを移動する。
//! 6. ファイルを削除し、ゴミ箱内のファイルの情報を取得してから、ファイルを復元する。
//! 7. ファイルの内容を更新し、過去のバージョンの取得、昇格、削除および復元を実行する。
//!
//! BOXは、プレミアムアカウントのユーザーのファイルだけにバージョンを記録する。
//! したがって、プレミアムアカウントでない場合は、手順7のバージョンの操作でエラーが発生する。
//! サンプルがアップロードおよびコピーしたファイルは、サンプルの終了後もBOXに残る。
mod common;

use std::path::Path;

use tokio::io::{AsyncBufReadExt as _, BufReader};

use box_client::{
    BoxClient, CopyFileOptions, DeleteItemOptions, DownloadFileOptions, File, FileId, FileLock,
    FileUpdate, FolderId, ListFileVersionsOptions, MoveDestination, RestoreItemOptions,
    SharedLinkAccess, SharedLinkSettings, UpdateFileContentOptions, UploadFileOptions,
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
/// `common::setup`が分割アップロードに切り替えるファイルサイズを20MiBにするため、20MiBを超えるファイルは、分割アップロードのAPIでアップロードする。
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

    // 一時ディレクトリにファイルをダウンロードし、アップロードしたファイルと内容がバイト単位で一致することを確認
    // 一時ディレクトリは、サンプルの終了時に削除する。
    let download_dir = tempfile::tempdir()?;
    let download_path = download_dir.path().join(file_name);
    let size = client
        .download_file(&file_id, &download_path, &DownloadFileOptions::default())
        .await?;
    tracing::info!(
        "downloaded file: path={}, size={size}",
        download_path.display()
    );
    if let Some(offset) = first_difference(file_path, &download_path).await? {
        anyhow::bail!("the downloaded file differs from the uploaded file at byte {offset}");
    }
    tracing::info!("the downloaded file is identical to the uploaded file");

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

    update_file_attributes(&client, &file_id, file.etag.as_deref()).await?;

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
    client
        .delete_file(&file_id, &DeleteItemOptions::default())
        .await?;
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
    let mut delete_options = DeleteItemOptions::default();
    if let Some(etag) = file.etag.as_deref() {
        delete_options = delete_options.with_etag(etag);
    }
    client
        .delete_file_version(file_id, &previous_version_id, &delete_options)
        .await?;
    tracing::info!("file version was deleted successfully");

    // 削除したバージョンを、ファイルのバージョン履歴に戻す
    let version = client
        .restore_file_version(file_id, &previous_version_id)
        .await?;
    tracing::info!("restored file version: {version:?}");

    Ok(())
}

/// 2つのファイルの内容を先頭からバイト単位で比較し、最初に異なるバイトの位置を返す。
///
/// 大きいファイルでもメモリを使い切らないように、ファイル全体を読み込まずに、バッファに読み込んだ範囲ごとに比較する。
/// 一方のファイルが他方のファイルの先頭部分と一致する場合は、短い方のファイルの長さを異なる位置として返す。
/// 内容が完全に一致する場合は`None`を返す。
async fn first_difference(left: &Path, right: &Path) -> std::io::Result<Option<u64>> {
    let mut left = BufReader::new(tokio::fs::File::open(left).await?);
    let mut right = BufReader::new(tokio::fs::File::open(right).await?);
    let mut offset: u64 = 0;
    loop {
        let left_buf = left.fill_buf().await?;
        let right_buf = right.fill_buf().await?;
        if left_buf.is_empty() || right_buf.is_empty() {
            // 両方のファイルを最後まで読んだ場合のみ、内容が一致する。
            return Ok((left_buf.len() != right_buf.len()).then_some(offset));
        }

        // 2つのバッファに読み込まれた長さは異なりうるため、短い方の長さだけ比較する。
        let len = left_buf.len().min(right_buf.len());
        if let Some(index) = left_buf[..len]
            .iter()
            .zip(&right_buf[..len])
            .position(|(l, r)| l != r)
        {
            return Ok(Some(offset + index as u64));
        }
        left.consume(len);
        right.consume(len);
        offset += len as u64;
    }
}

/// ファイルの複数の属性を1回のリクエストでまとめて更新してから、共有リンクなどを削除する。
async fn update_file_attributes(
    client: &BoxClient,
    file_id: &FileId,
    etag: Option<&str>,
) -> anyhow::Result<File> {
    // ファイルの複数の属性を1回のリクエストで更新
    // タグと共有リンクを設定し、ファイルをロックする。指定しない属性は送信せず、BOXは変更しない。
    let tags = ["box-client", "example"];
    let update = FileUpdate::default()
        .with_tags(&tags)
        .with_shared_link(
            SharedLinkSettings::default().with_access(SharedLinkAccess::Collaborators),
        )
        .with_lock(FileLock::default().with_is_download_prevented(false));
    let file = client
        .update_file(file_id, &update, &common::update_options(etag))
        .await?;
    tracing::info!("updated file: etag={:?}", file.etag);

    // 共有リンクを削除し、ロックを解除
    // 削除する属性には、BOXにnullを送信する。
    let update = FileUpdate::default()
        .with_shared_link_removed()
        .with_lock_removed();
    let file = client
        .update_file(
            file_id,
            &update,
            &common::update_options(file.etag.as_deref()),
        )
        .await?;
    tracing::info!("removed file shared link and lock: etag={:?}", file.etag);

    Ok(file)
}
