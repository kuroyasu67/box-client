mod common;

use box_client::FileId;

/// コマンドライン引数で指定したIDのファイルを削除する。
///
/// 実行例: `cargo run --example delete_file -- 12345`
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let file_id = std::env::args()
        .nth(1)
        .map(FileId::new)
        .ok_or_else(|| anyhow::anyhow!("usage: delete_file <FILE_ID>"))?;

    let client = common::setup().await?;

    // 削除する直前のETagを指定し、ETagの取得後にファイルが変更されていた場合は削除しない
    let file = client.file_information(&file_id).await?;
    client.delete_file(&file_id, file.etag.as_deref()).await?;
    tracing::info!("deleted file: id={}, name={}", file.id, file.name);

    common::list_trashed_items(&client).await
}
