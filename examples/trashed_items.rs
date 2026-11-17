mod common;

/// ゴミ箱内のすべての項目を表示する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = common::setup().await?;

    common::list_trashed_items(&client).await
}
