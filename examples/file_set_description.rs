//! ファイルの説明を設定するサンプル
mod common;

use box_client::{FileId, UpdateItemOptions};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=file_set_description -- <file-id> <description>";

/// ファイルの説明を設定する。
///
/// 説明を削除する場合は、`<description>`に空文字列（`""`）を指定する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 説明を設定するファイルのIDと説明を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_id, description) = match args.as_slice() {
        [file_id, description] => (FileId::new(file_id), description.clone()),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    // ファイルの最新のETagを取得し、取得後にファイルが変更されていた場合は説明を設定しない。
    let file = client.file_information(&file_id).await?;
    let mut options = UpdateItemOptions::default();
    if let Some(etag) = file.etag.as_deref() {
        options = options.with_etag(etag);
    }

    let file = client
        .set_file_description(&file_id, &description, &options)
        .await?;
    tracing::info!("set file description: {:?}", file.description);

    Ok(())
}
