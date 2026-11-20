//! ファイルおよびフォルダを1つのZIPファイルにまとめてダウンロードするサンプル
//!
//! サンプルは、次の順にZIPダウンロードを操作する。
//!
//! 1. ZIPダウンロードを作成し、ZIPファイルをダウンロードしてから、ダウンロードを終えた後の状態を取得する
//!    （`BoxClient::download_zip`）。
//! 2. 同じ項目でZIPダウンロードを作成し直し、作成、ダウンロードおよび状態の取得を1つずつ呼び出す。
//!
//! ダウンロード用のURLは一度しか使用できないため、2回目のダウンロードでは、ZIPダウンロードを作成し直す。
mod common;

use std::path::Path;

use box_client::{CreateZipDownloadOptions, FileId, FolderId, ZipDownloadItem};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=zip_download -- <output-dir> <file:ID|folder:ID>...";

/// ファイルおよびフォルダを1つのZIPファイルにまとめてダウンロードする。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=zip_download -- <output-dir> <file:ID|folder:ID>...
/// ```
///
/// `file:12345`または`folder:67890`の形式で、ZIPファイルに含めるファイルおよびフォルダを1つ以上指定する。
/// サンプルは、`<output-dir>`ディレクトリに2つのZIPファイルを保存する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 保存先のディレクトリと、ZIPファイルに含める項目のIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((output_dir, item_args)) = args.split_first() else {
        anyhow::bail!(USAGE);
    };
    if item_args.is_empty() {
        anyhow::bail!(USAGE);
    }
    let mut file_ids = Vec::new();
    let mut folder_ids = Vec::new();
    for arg in item_args {
        match arg.split_once(':') {
            Some(("file", id)) => file_ids.push(FileId::new(id)),
            Some(("folder", id)) => folder_ids.push(FolderId::new(id)),
            _ => anyhow::bail!(USAGE),
        }
    }
    let items: Vec<ZipDownloadItem<'_>> = file_ids
        .iter()
        .map(ZipDownloadItem::File)
        .chain(folder_ids.iter().map(ZipDownloadItem::Folder))
        .collect();
    let output_dir = Path::new(output_dir);

    let client = common::setup().await?;
    let prefix = common::name_prefix();

    // ZIPダウンロードの作成、ダウンロードおよび状態の取得をまとめて実行
    // BOXは、指定した名前の末尾に`.zip`を付ける。
    let options = CreateZipDownloadOptions::default().with_download_file_name(&prefix);
    let zip_path = output_dir.join(format!("{prefix}.zip"));
    let summary = client.download_zip(&items, &zip_path, &options).await?;
    tracing::info!(
        "downloaded zip file: path={}, size={}",
        zip_path.display(),
        summary.size
    );
    for conflict in summary.name_conflicts.iter().flatten() {
        tracing::info!(
            "renamed in zip file: {} -> {}",
            conflict.original_name,
            conflict.download_name
        );
    }
    match &summary.status {
        Ok(status) => tracing::info!("zip download status: {status:?}"),
        // 状態を取得できなくても、ZIPファイルは保存済みである。
        Err(error) => tracing::warn!("failed to get zip download status: {error}"),
    }

    // ZIPダウンロードを作成
    // ダウンロード用のURLは数秒しか有効でないため、作成したらすぐにダウンロードする。
    let zip_download = client.create_zip_download(&items, &options).await?;
    tracing::info!("created zip download: {zip_download:?}");

    // ZIPファイルをダウンロード
    // BOXはダウンロード用のURLで認証を求めないため、BOXクライアントはアクセストークンを送信しない。
    let zip_path = output_dir.join(format!("{prefix}-2.zip"));
    let size = client
        .download_zip_content(&zip_download, &zip_path)
        .await?;
    tracing::info!(
        "downloaded zip file: path={}, size={size}",
        zip_path.display()
    );

    // ダウンロードを終えた後の状態を取得
    // BOXは、ダウンロードを始めてから12時間、状態を返す。
    let status = client.zip_download_status(&zip_download).await?;
    tracing::info!(
        "zip download status: state={:?}, downloaded={:?}, skipped_files={:?}, skipped_folders={:?}",
        status.state,
        status.downloaded_file_count,
        status.skipped_file_count,
        status.skipped_folder_count
    );

    Ok(())
}
