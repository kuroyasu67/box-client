//! ゴミ箱内のすべての項目を表示するサンプル

mod common;

/// ゴミ箱内のすべての項目を表示する。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=trashed_items
/// ```
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = common::setup().await?;

    common::list_trashed_items(&client).await
}
