mod common;

/// BOX設定ファイルを読み込んで認証し、サービスアカウントのユーザー情報を表示する。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=me
/// ```
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = common::setup().await?;

    let user = client.me().await?;
    tracing::info!("id={}, name={}, login={}", user.id, user.name, user.login);

    Ok(())
}
