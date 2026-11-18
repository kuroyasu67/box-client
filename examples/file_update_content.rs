//! ファイルの内容を更新するサンプル
mod common;

use box_client::{FileId, UpdateFileContentOptions};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=file_update_content -- <file-id> <file-path> [<new-name>]";

/// ファイルの内容を更新する。
///
/// `<new-name>`を指定した場合は、内容の更新とともにファイル名を変更する。
/// 50MiBを超えるファイルは、分割アップロードのAPIでアップロードする。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 内容を更新するファイルのID、新しい内容のファイルのパス、および新しいファイル名を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_id, file_path, name) = match args.as_slice() {
        [file_id, file_path] => (FileId::new(file_id), file_path, None),
        [file_id, file_path, name] => (FileId::new(file_id), file_path, Some(name.as_str())),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    // ファイルの最新のETagを取得し、取得後にファイルが変更されていた場合は内容を更新しない。
    let file = client.file_information(&file_id).await?;
    let mut options = UpdateFileContentOptions::default();
    if let Some(etag) = file.etag.as_deref() {
        options = options.with_etag(etag);
    }
    if let Some(name) = name {
        options = options.with_name(name);
    }

    let file = client
        .update_file_content(&file_id, file_path, &options)
        .await?;
    tracing::info!("updated file: {file:?}");

    Ok(())
}
