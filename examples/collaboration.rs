//! コラボレーションを操作するメソッドを順に呼び出すサンプル
//!
//! サンプルは、次の順にコラボレーションを操作する。
//!
//! 1. コラボレーションの対象にするフォルダを作成し、フォルダにファイルをアップロードする。
//! 2. フォルダに、メールアドレスで指定したユーザーのコラボレーションを作成し、情報を取得する。
//! 3. フォルダおよびファイルのコラボレーションをリストする。
//! 4. コラボレーションの役割を更新してから、コラボレーションを削除する。
//! 5. サービスアカウント宛ての保留中のコラボレーションの招待をリストする。
//! 6. グループのIDを指定した場合は、グループのコラボレーションをリストする。
//!
//! サンプルは、コラボレーションを作成するときに招待の電子メールを送信しない。
//! また、元に戻せないため、項目の所有者の変更（[`CollaborationRole::Owner`]への更新）は実行しない。
//! サンプルが作成したフォルダおよびファイルは、サンプルの終了後もBOXに残る。
mod common;

use box_client::{
    BoxClient, Collaboration, CollaborationGrantee, CollaborationItem, CollaborationRole,
    CollaborationUpdate, CollaborationUpdateOutcome, Collaborator, CreateCollaborationOptions,
    FileId, FolderId, GroupId, ListFileCollaborationsOptions, ListFolderCollaborationsOptions,
    ListGroupCollaborationsOptions, ListPendingCollaborationsOptions, UploadFileOptions,
};

/// 使用方法
const USAGE: &str = "usage: RUST_LOG=info cargo run --quiet --example=collaboration -- <parent-folder-id> <collaborator-email> [<group-id>]";

/// コラボレーションを操作するメソッドを順に呼び出す。
///
/// ```shell
/// RUST_LOG=info cargo run --quiet --example=collaboration -- <parent-folder-id> <collaborator-email> [<group-id>]
/// ```
///
/// `<parent-folder-id>`のフォルダにフォルダを作成し、そのフォルダに`<collaborator-email>`のユーザーのコラボレーションを作成する。
/// `<collaborator-email>`には、サービスアカウント以外のユーザーのメールアドレスを指定する。
///
/// サービスアカウントを、`<parent-folder-id>`のフォルダの所有者または共同所有者にしておく。
/// サービスアカウントが他のユーザーのフォルダの編集者である場合、サンプルが作成したフォルダの所有者は親フォルダの所有者になる。
/// このとき、所有者と共同所有者だけが招待できるように設定されていると、BOXはコラボレーションの作成に403
/// （`access_denied_insufficient_permissions`）を返す。
///
/// `<group-id>`を指定した場合は、そのグループのコラボレーションをリストする。
/// BOXのドキュメントによると、グループのコラボレーションをリストするには、管理者の権限が必要である。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 親フォルダのID、コラボレーターのメールアドレス、およびグループのIDを取得
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (parent_id, collaborator_email, group_id) = match args.as_slice() {
        [parent_id, email] => (FolderId::new(parent_id), email, None),
        [parent_id, email, group_id] => (
            FolderId::new(parent_id),
            email,
            Some(GroupId::new(group_id)),
        ),
        _ => anyhow::bail!(USAGE),
    };

    let client = common::setup().await?;
    let prefix = common::name_prefix();

    // コラボレーションの対象にするフォルダを作成し、フォルダにファイルをアップロード
    let folder = client.create_folder(&prefix, &parent_id).await?;
    tracing::info!("created folder: id={}, name={}", folder.id, folder.name);
    let file_id = upload_file(&client, &folder.id, &prefix).await?;

    // フォルダにコラボレーションを作成
    // 招待の電子メールを送信しないように、通知を無効にする。
    let collaboration = client
        .create_collaboration(
            CollaborationItem::Folder(&folder.id),
            Collaborator::UserLogin(collaborator_email),
            CollaborationRole::Viewer,
            &CreateCollaborationOptions::default().with_notify(false),
        )
        .await?;
    tracing::info!("created collaboration:");
    log_collaboration(&collaboration);
    let collaboration_id = collaboration.id;

    // コラボレーションの情報を取得
    let collaboration = client.collaboration_information(&collaboration_id).await?;
    tracing::info!("collaboration information: {collaboration:?}");

    // フォルダのすべてのコラボレーションをリスト
    let collaborations = client
        .list_all_folder_collaborations(&folder.id, &ListFolderCollaborationsOptions::default())
        .await?;
    tracing::info!("folder collaborations: {}", collaborations.len());
    collaborations.iter().for_each(log_collaboration);

    // ファイルのすべてのコラボレーションをリスト
    let collaborations = client
        .list_all_file_collaborations(&file_id, &ListFileCollaborationsOptions::default())
        .await?;
    tracing::info!("file collaborations: {}", collaborations.len());
    collaborations.iter().for_each(log_collaboration);

    // コラボレーションの役割を更新
    // 役割に`CollaborationRole::Owner`を指定すると項目の所有者を変更し、BOXは更新後のコラボレーションを返さない。
    let outcome = client
        .update_collaboration(
            &collaboration_id,
            &CollaborationUpdate::default().with_role(CollaborationRole::Editor),
        )
        .await?;
    // `CollaborationUpdateOutcome`は`#[non_exhaustive]`であるため、所有者の変更と将来追加される結果をまとめて扱う。
    if let CollaborationUpdateOutcome::Updated(collaboration) = outcome {
        tracing::info!("updated collaboration:");
        log_collaboration(&collaboration);
    } else {
        tracing::info!("owner of the folder was changed");
    }

    // コラボレーションを削除
    client.delete_collaboration(&collaboration_id).await?;
    tracing::info!("collaboration was deleted successfully");

    // サービスアカウント宛ての保留中のコラボレーションの招待をリスト
    let invitations = client
        .list_all_pending_collaborations(&ListPendingCollaborationsOptions::default())
        .await?;
    tracing::info!("pending collaborations: {}", invitations.len());
    invitations.iter().for_each(log_collaboration);

    // グループのすべてのコラボレーションをリスト
    if let Some(group_id) = group_id {
        let collaborations = client
            .list_all_group_collaborations(&group_id, &ListGroupCollaborationsOptions::default())
            .await?;
        tracing::info!("group collaborations: {}", collaborations.len());
        collaborations.iter().for_each(log_collaboration);
    }

    Ok(())
}

/// 一時ファイルを作成して、フォルダにアップロードし、アップロードしたファイルのIDを返す。
async fn upload_file(
    client: &BoxClient,
    folder_id: &FolderId,
    prefix: &str,
) -> anyhow::Result<FileId> {
    // 一時ディレクトリは、この関数の終了時に削除する。
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("collaboration.txt");
    tokio::fs::write(&path, "box-clientのサンプルがアップロードしたファイル").await?;

    let name = format!("{prefix}.txt");
    let file = client
        .upload_file(
            &path,
            folder_id,
            &UploadFileOptions::default().with_name(&name),
        )
        .await?;
    tracing::info!("uploaded file: id={}, name={}", file.id, file.name);

    Ok(file.id)
}

/// コラボレーションのID、役割、状態およびアクセス権限を与えた相手を表示する。
fn log_collaboration(collaboration: &Collaboration) {
    let grantee = match &collaboration.accessible_by {
        Some(CollaborationGrantee::User(user)) => format!(
            "user(id={}, login={})",
            user.id,
            user.login.as_deref().unwrap_or_default()
        ),
        Some(CollaborationGrantee::Group(group)) => format!(
            "group(id={}, name={})",
            group.id,
            group.name.as_deref().unwrap_or_default()
        ),
        // `CollaborationGrantee`は`#[non_exhaustive]`であるため、`CollaborationGrantee::Unknown`と将来追加される種類をまとめて扱う。
        Some(_) => "unknown".to_owned(),
        None => "none".to_owned(),
    };
    tracing::info!(
        "collaboration: id={}, role={:?}, status={:?}, accessible_by={grantee}",
        collaboration.id,
        collaboration.role,
        collaboration.status
    );
}
