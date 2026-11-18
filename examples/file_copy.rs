//! ファイルをコピーするサンプル
mod common;

use box_client::{CopyFileOptions, FileId, FolderId};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=file_copy -- <file-id> <parent-folder-id> [<new-name>]";

/// ファイルをコピーする。
///
/// `<new-name>`を省略した場合、BOXはコピー元のファイルと同じ名前を付ける。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // コピーするファイルのID、コピー先の親フォルダのID、およびコピーしたファイルの名前を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file_id, parent_id, name) = match args.as_slice() {
        [file_id, parent_id] => (FileId::new(file_id), FolderId::new(parent_id), None),
        [file_id, parent_id, name] => (
            FileId::new(file_id),
            FolderId::new(parent_id),
            Some(name.as_str()),
        ),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;

    let mut options = CopyFileOptions::default();
    if let Some(name) = name {
        options = options.with_name(name);
    }

    let file = client.copy_file(&file_id, &parent_id, &options).await?;
    tracing::info!("copied file: {file:?}");

    Ok(())
}
