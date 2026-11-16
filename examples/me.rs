mod common;

/// Box設定ファイルを読み込んで認証し、サービスアカウントのユーザー情報を表示する。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = common::setup().await?;

    let user = client.me().await?;
    tracing::info!("id={}, name={}, login={}", user.id, user.name, user.login);

    Ok(())
}
