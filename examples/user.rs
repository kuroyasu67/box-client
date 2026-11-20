//! ユーザーを操作するメソッドを順に呼び出すサンプル
//!
//! サンプルは、次の順にユーザーを操作する。
//!
//! 1. 言語が日本語でタイムゾーンが日本時間の管理対象ユーザーを作成し、情報を取得する。
//! 2. メールアドレスを検索語にして、作成した管理対象ユーザーを検索する。
//! 3. 管理対象ユーザーの役職および状態を更新する。
//! 4. 管理対象ユーザーを削除する。
//! 5. 検索語を指定した場合は、名前またはメールアドレスが検索語で始まるエンタープライズのユーザーをリストする。
//!
//! 管理対象ユーザーは、エンタープライズのライセンスを1つ使用する。
//! また、BOXは、作成した管理対象ユーザーのメールアドレスに、通知を送信する場合がある。
//! 作成した管理対象ユーザーは、サンプルの終了前に削除する。削除したことは、ユーザーに通知しない。
//! ユーザーの作成、更新および削除には、アプリケーションに「ユーザーを管理する」のスコープが必要である。
mod common;

use box_client::{
    CreateUserOptions, DeleteUserOptions, ListEnterpriseUsersOptions, NewUserAccount, User,
    UserStatus, UserTypeFilter, UserUpdate,
};

/// 使用方法
const USAGE: &str =
    "usage: RUST_LOG=info cargo run --quiet --example=user -- <login> [<filter-term>]";

/// ユーザーを操作するメソッドを順に呼び出す。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=user -- <login> [<filter-term>]
/// ```
///
/// `<login>`をログインに使用するメールアドレスとして、管理対象ユーザーを作成する。
/// `<login>`には、BOXに登録していないメールアドレスを指定する。
/// `<filter-term>`を指定した場合は、名前またはメールアドレスが`<filter-term>`で始まる管理対象ユーザーとアプリユーザーをリストする。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ログインに使用するメールアドレスと検索語を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (login, filter_term) = match args.as_slice() {
        [login] => (login.as_str(), None),
        [login, filter_term] => (login.as_str(), Some(filter_term.as_str())),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;
    // BOXはユーザー名を50文字以下に制限するため、接頭辞をそのままユーザー名にする。
    let name = common::name_prefix();

    // 言語が日本語でタイムゾーンが日本時間の管理対象ユーザーを作成
    let user = client
        .create_user(
            &name,
            NewUserAccount::Managed { login },
            &CreateUserOptions::default()
                .with_language("ja")
                .with_timezone("Asia/Tokyo"),
        )
        .await?;
    tracing::info!("created managed user:");
    log_user(&user);
    let user_id = user.id;

    // ユーザー情報を取得
    let user = client.user_information(&user_id).await?;
    tracing::info!("user information: {user:?}");

    // メールアドレスを検索語にして、作成した管理対象ユーザーを検索
    let users = client
        .list_all_enterprise_users(
            &ListEnterpriseUsersOptions::default()
                .with_filter_term(login)
                .with_user_type(UserTypeFilter::Managed),
        )
        .await?;
    tracing::info!("users starting with the login: {}", users.len());
    users.iter().for_each(log_user);

    // ユーザーを更新
    let user = client
        .update_user(
            &user_id,
            &UserUpdate::default()
                .with_job_title("box-clientのサンプル")
                .with_status(UserStatus::Inactive),
        )
        .await?;
    tracing::info!("updated user:");
    log_user(&user);

    // ユーザーを削除
    // 作成したばかりのユーザーはコンテンツを所有していないが、所有していても削除できるように`force`を指定する。
    client
        .delete_user(
            &user_id,
            &DeleteUserOptions::default()
                .with_notify(false)
                .with_force(true),
        )
        .await?;
    tracing::info!("user was deleted successfully");

    // 名前またはメールアドレスが検索語で始まるユーザーをリスト
    if let Some(filter_term) = filter_term {
        let users = client
            .list_all_enterprise_users(
                &ListEnterpriseUsersOptions::default()
                    .with_filter_term(filter_term)
                    .with_user_type(UserTypeFilter::Managed),
            )
            .await?;
        tracing::info!("users starting with {filter_term:?}: {}", users.len());
        users.iter().for_each(log_user);
    }

    Ok(())
}

/// ユーザーのID、名前、メールアドレス、言語、タイムゾーン、状態および役職を表示する。
fn log_user(user: &User) {
    tracing::info!(
        "user: id={}, name={}, login={}, language={}, timezone={}, status={:?}, job_title={}",
        user.id,
        user.name.as_deref().unwrap_or_default(),
        user.login.as_deref().unwrap_or_default(),
        user.language.as_deref().unwrap_or_default(),
        user.timezone.as_deref().unwrap_or_default(),
        user.status,
        user.job_title.as_deref().unwrap_or_default()
    );
}
