//! グループとグループメンバーシップを操作するメソッドを順に呼び出すサンプル
//!
//! サンプルは、次の順にグループとグループメンバーシップを操作する。
//!
//! 1. グループを作成し、情報を取得する。
//! 2. グループ名を検索語にして、作成したグループを検索する。
//! 3. グループ名、説明および操作できるユーザーの範囲を更新する。
//! 4. メールアドレスで指定したユーザーを、メンバーとしてグループに追加し、メンバーシップの情報を取得する。
//! 5. グループのメンバーと、ユーザーが所属するグループをリストする。
//! 6. ユーザーの役割をグループの管理者に変更し、管理者に与える権限を設定する。
//! 7. ユーザーをグループから外し、グループを削除する。
//! 8. 検索語を指定した場合は、グループ名が検索語で始まるエンタープライズのグループをリストする。
//!
//! 作成したグループとメンバーシップは、サンプルの終了前に削除する。
//! ただし、途中でエラーになった場合は、作成したグループがBOXに残る。
//! グループとメンバーシップの作成および削除には、アプリケーションに「グループを管理する」のスコープが必要である。
//! また、メールアドレスからユーザーを探すため、「ユーザーを管理する」のスコープが必要である。
mod common;

use box_client::{
    BoxClient, CreateGroupMembershipOptions, CreateGroupOptions, Group, GroupAccessLevel,
    GroupMembership, GroupMembershipRole, GroupMembershipUpdate, GroupUpdate,
    ListEnterpriseGroupsOptions, ListEnterpriseUsersOptions, ListGroupMembershipsOptions, UserId,
    UserTypeFilter,
};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=group_membership -- <user-login> [<filter-term>]";

/// グループとグループメンバーシップを操作するメソッドを順に呼び出す。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=group_membership -- <user-login> [<filter-term>]
/// ```
///
/// `<user-login>`には、グループに追加する管理対象ユーザーのメールアドレスを指定する。
/// `<filter-term>`を指定した場合は、グループ名が`<filter-term>`で始まるグループをリストする。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // グループに追加するユーザーのメールアドレスと検索語を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (user_login, filter_term) = match args.as_slice() {
        [user_login] => (user_login.as_str(), None),
        [user_login, filter_term] => (user_login.as_str(), Some(filter_term.as_str())),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;
    // グループに追加するユーザーのIDを、メールアドレスから探す。
    let user_id = find_user_id(&client, user_login).await?;
    // グループ名はエンタープライズで一意でなければならないため、実行ごとに異なる接頭辞をグループ名にする。
    let name = common::name_prefix();

    // グループを作成
    let group = client
        .create_group(
            &name,
            &CreateGroupOptions::default()
                .with_description("box-clientのサンプルが作成したグループ")
                .with_invitability_level(GroupAccessLevel::AdminsOnly)
                .with_member_viewability_level(GroupAccessLevel::AdminsOnly),
        )
        .await?;
    tracing::info!("created group:");
    log_group(&group);
    let group_id = group.id;

    // グループ情報を取得
    let group = client.group_information(&group_id).await?;
    tracing::info!("group information: {group:?}");

    // グループ名を検索語にして、作成したグループを検索
    let groups = client
        .list_all_enterprise_groups(&ListEnterpriseGroupsOptions::default().with_filter_term(&name))
        .await?;
    tracing::info!("groups starting with the name: {}", groups.len());
    groups.iter().for_each(log_group);

    // グループを更新
    let new_name = format!("{name}-updated");
    let group = client
        .update_group(
            &group_id,
            &GroupUpdate::default()
                .with_name(&new_name)
                .with_description("box-clientのサンプルが更新したグループ")
                .with_invitability_level(GroupAccessLevel::AdminsAndMembers)
                .with_member_viewability_level(GroupAccessLevel::AdminsAndMembers),
        )
        .await?;
    tracing::info!("updated group:");
    log_group(&group);

    // ユーザーを、メンバーとしてグループに追加
    let membership = client
        .create_group_membership(
            &user_id,
            &group_id,
            &CreateGroupMembershipOptions::default().with_role(GroupMembershipRole::Member),
        )
        .await?;
    tracing::info!("created group membership:");
    log_membership(&membership);
    let membership_id = membership.id;

    // メンバーシップの情報を取得
    let membership = client.group_membership_information(&membership_id).await?;
    tracing::info!("group membership information: {membership:?}");

    // グループのメンバーをリスト
    let memberships = client
        .list_all_group_memberships(&group_id, &ListGroupMembershipsOptions::default())
        .await?;
    tracing::info!("members of group: {}", memberships.len());
    memberships.iter().for_each(log_membership);

    // ユーザーが所属するグループをリスト
    let memberships = client
        .list_all_user_memberships(&user_id, &ListGroupMembershipsOptions::default())
        .await?;
    tracing::info!("groups of user: {}", memberships.len());
    memberships.iter().for_each(log_membership);

    // ユーザーの役割をグループの管理者に変更し、レポートを実行する権限だけを与えない。
    let membership = client
        .update_group_membership(
            &membership_id,
            &GroupMembershipUpdate::default()
                .with_role(GroupMembershipRole::Admin)
                .with_configurable_permissions(&[("can_run_reports", false)]),
        )
        .await?;
    tracing::info!("updated group membership:");
    log_membership(&membership);

    // ユーザーをグループから外す
    client.delete_group_membership(&membership_id).await?;
    tracing::info!("group membership was deleted successfully");

    // グループを削除
    client.delete_group(&group_id).await?;
    tracing::info!("group was deleted successfully");

    // グループ名が検索語で始まるグループをリスト
    if let Some(filter_term) = filter_term {
        let groups = client
            .list_all_enterprise_groups(
                &ListEnterpriseGroupsOptions::default().with_filter_term(filter_term),
            )
            .await?;
        tracing::info!("groups starting with {filter_term:?}: {}", groups.len());
        groups.iter().for_each(log_group);
    }

    Ok(())
}

/// メールアドレスが一致する管理対象ユーザーのIDを返す。
///
/// BOXは、名前またはメールアドレスが検索語で始まるユーザーを返すため、メールアドレスが完全に一致するユーザーを選ぶ。
async fn find_user_id(client: &BoxClient, login: &str) -> anyhow::Result<UserId> {
    let users = client
        .list_all_enterprise_users(
            &ListEnterpriseUsersOptions::default()
                .with_filter_term(login)
                .with_user_type(UserTypeFilter::Managed),
        )
        .await?;
    users
        .into_iter()
        .find(|user| {
            user.login
                .as_deref()
                .is_some_and(|l| l.eq_ignore_ascii_case(login))
        })
        .map(|user| user.id)
        .ok_or_else(|| anyhow::anyhow!("user with the login was not found: {login}"))
}

/// グループのID、名前、種類、説明および操作できるユーザーの範囲を表示する。
fn log_group(group: &Group) {
    tracing::info!(
        "group: id={}, name={}, group_type={:?}, description={}, invitability_level={:?}, member_viewability_level={:?}",
        group.id,
        group.name.as_deref().unwrap_or_default(),
        group.group_type,
        group.description.as_deref().unwrap_or_default(),
        group.invitability_level,
        group.member_viewability_level
    );
}

/// メンバーシップのID、ユーザー、グループおよび役割を表示する。
fn log_membership(membership: &GroupMembership) {
    tracing::info!(
        "membership: id={}, user={}, group={}, role={:?}",
        membership.id,
        membership
            .user
            .as_ref()
            .map(|user| format!("{}({})", user.id, user.login.as_deref().unwrap_or_default()))
            .unwrap_or_default(),
        membership
            .group
            .as_ref()
            .map(|group| format!(
                "{}({})",
                group.id,
                group.name.as_deref().unwrap_or_default()
            ))
            .unwrap_or_default(),
        membership.role
    );
}
