//! グループを操作するメソッドを順に呼び出すサンプル
//!
//! サンプルは、次の順にグループを操作する。
//!
//! 1. グループを作成し、情報を取得する。
//! 2. グループ名を検索語にして、作成したグループを検索する。
//! 3. グループ名、説明および操作できるユーザーの範囲を更新する。
//! 4. グループを削除する。
//! 5. 検索語を指定した場合は、グループ名が検索語で始まるエンタープライズのグループをリストする。
//!
//! 作成したグループは、サンプルの終了前に削除する。
//! グループの作成および削除には、アプリケーションに「グループを管理する」のスコープが必要である。
mod common;

use box_client::{
    CreateGroupOptions, Group, GroupAccessLevel, GroupUpdate, ListEnterpriseGroupsOptions,
};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=group -- [<filter-term>]";

/// グループを操作するメソッドを順に呼び出す。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=group -- [<filter-term>]
/// ```
///
/// `<filter-term>`を指定した場合は、グループ名が`<filter-term>`で始まるグループをリストする。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 検索語を取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let filter_term = match args.as_slice() {
        [] => None,
        [filter_term] => Some(filter_term.as_str()),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;
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
