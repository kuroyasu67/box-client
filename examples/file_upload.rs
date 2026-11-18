//! ファイルをアップロードするサンプル
mod common;

use box_client::{FolderId, UploadFileOptions};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=file_upload -- <file-path> <parent-folder-id> [<name>]";

/// ファイルをアップロードする。
///
/// `<name>`を省略した場合は、`<file-path>`のファイル名でアップロードする。
/// 50MiBを超えるファイルは、分割アップロードのAPIでアップロードする。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // アップロードするファイルのパス、アップロード先の親フォルダのID、およびファイル名を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_path, parent_id, name) = match args.as_slice() {
        [file_path, parent_id] => (file_path, FolderId::new(parent_id), None),
        [file_path, parent_id, name] => (file_path, FolderId::new(parent_id), Some(name.as_str())),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    let mut options = UploadFileOptions::default();
    if let Some(name) = name {
        options = options.with_name(name);
    }

    let file = client.upload_file(file_path, &parent_id, &options).await?;
    tracing::info!("uploaded file: {file:?}");

    Ok(())
}
