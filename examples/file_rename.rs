//! ファイル名を変更するサンプル
mod common;

use box_client::{FileId, UpdateItemOptions};

/// 使用方法
const USAGE: &str =
    "usage: RUST_LOG=info cargo run --quiet --example=file_rename -- <file-id> <new-name>";

/// ファイル名を変更する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 名前を変更するファイルのIDと新しいファイル名を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_id, name) = match args.as_slice() {
        [file_id, name] => (FileId::new(file_id), name.clone()),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    // ファイルの最新のETagを取得し、取得後にファイルが変更されていた場合は名前を変更しない。
    let file = client.file_information(&file_id).await?;
    let mut options = UpdateItemOptions::default();
    if let Some(etag) = file.etag.as_deref() {
        options = options.with_etag(etag);
    }

    let file = client.rename_file(&file_id, &name, &options).await?;
    tracing::info!("renamed file: {file:?}");

    Ok(())
}
