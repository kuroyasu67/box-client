//! コラボレーションに関するAPI
//!
//! コラボレーションは、ユーザーまたはグループに、フォルダ、ファイルまたはアプリ項目へのアクセス権限を与える。

use serde::Deserialize;
use time::OffsetDateTime;

use crate::{
    AppItemId, BoxClient, BoxClientResult, BoxItem, CollaborationId, FileId, FolderId, GroupId,
    TermsOfServiceId, UserId,
    api::pagination::{MAX_ITEMS_LIMIT, MarkerPage, collect_all_pages, marker_page_request},
    request::ApiRequest,
};

/// コラボレーション情報
///
/// BOXのAPI仕様（OpenAPI）は、`id`と`type`だけを必須としている。
/// BOXが属性を省略したりnullを返したりしても取得を失敗させないように、`id`以外の属性を`Option`で受け取る。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Collaboration {
    /// コラボレーションID
    pub id: CollaborationId,

    /// コラボレーションの対象のファイル、フォルダまたはWebリンク
    ///
    /// コラボレーションの状態が[`CollaborationStatus::Pending`]の場合、
    /// またはコラボレーションの対象がアプリ項目の場合は`None`となる。
    pub item: Option<BoxItem>,

    /// コラボレーションの対象のアプリ項目
    ///
    /// コラボレーションの対象がファイル、フォルダまたはWebリンクの場合、
    /// またはアプリ項目にアクセスできない場合は`None`となる。
    /// 与えた役割は、アプリ項目に含まれるすべての項目に適用される。
    pub app_item: Option<AppItem>,

    /// アクセス権限を与えたユーザーまたはグループ
    pub accessible_by: Option<CollaborationGrantee>,

    /// BOXに登録していないユーザーを招待したときに使用したメールアドレス
    pub invite_email: Option<String>,

    /// 与えた役割
    pub role: Option<CollaborationRole>,

    /// コラボレーションの有効期限
    ///
    /// 有効期限を設定していない場合は`None`となる。
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,

    /// コラボレーターが、「すべてのファイル」に表示されない項目にアクセスできるかどうか
    ///
    /// `true`の場合、コラボレーターは、「すべてのファイル」に表示されない項目にアクセスでき、
    /// ルートフォルダからのパスを参照できない。
    pub is_access_only: Option<bool>,

    /// コラボレーションの状態
    pub status: Option<CollaborationStatus>,

    /// コラボレーションの状態が承認済みまたは拒否済みに変わった日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub acknowledged_at: Option<OffsetDateTime>,

    /// コラボレーションを作成したユーザー
    pub created_by: Option<CollaborationUser>,

    /// コラボレーションを作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,

    /// コラボレーションを最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,

    /// コラボレーションを承認する前に満たす必要がある条件
    pub acceptance_requirements_status: Option<AcceptanceRequirementsStatus>,
}

/// コラボレーションで与える役割
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub enum CollaborationRole {
    /// 編集者
    #[serde(rename = "editor")]
    Editor,

    /// 閲覧者
    #[serde(rename = "viewer")]
    Viewer,

    /// プレビューア
    #[serde(rename = "previewer")]
    Previewer,

    /// アップローダー
    #[serde(rename = "uploader")]
    Uploader,

    /// プレビューア アップローダー
    #[serde(rename = "previewer uploader")]
    PreviewerUploader,

    /// 閲覧者 アップローダー
    #[serde(rename = "viewer uploader")]
    ViewerUploader,

    /// 共同所有者
    #[serde(rename = "co-owner")]
    CoOwner,

    /// 所有者
    #[serde(rename = "owner")]
    Owner,

    /// BOXクライアントが知らない役割
    ///
    /// BOXが役割を追加しても、コラボレーション情報の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

/// コラボレーションの状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CollaborationStatus {
    /// 承認済み
    Accepted,

    /// 承認待ち
    ///
    /// BOXは、承認待ちのコラボレーターの名前を空文字列で返す。
    Pending,

    /// 拒否済み
    Rejected,

    /// BOXクライアントが知らない状態
    ///
    /// BOXが状態を追加しても、コラボレーション情報の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

/// アクセス権限を与えたユーザーまたはグループ
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum CollaborationGrantee {
    /// ユーザー
    User(CollaborationUser),

    /// グループ
    Group(GroupMini),

    /// BOXクライアントが知らない種類
    ///
    /// BOXが種類を追加しても、コラボレーション情報の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

/// コラボレーションに含まれるユーザー
///
/// [`UserMini`](crate::UserMini)の属性に加えて、ユーザーが有効かどうかを持つ。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct CollaborationUser {
    /// ユーザーID
    pub id: UserId,

    /// 表示名
    ///
    /// コラボレーションの状態が[`CollaborationStatus::Pending`]の場合、BOXは空文字列を返す。
    pub name: Option<String>,

    /// ログインに使用するメールアドレス
    ///
    /// コラボレーションの状態が[`CollaborationStatus::Pending`]の場合も、BOXはメールアドレスを返す。
    pub login: Option<String>,

    /// ユーザーが有効かどうか
    ///
    /// ユーザーを無効にした場合、または削除した場合は`false`となる。
    pub is_active: Option<bool>,
}

/// 最小限のグループ情報
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct GroupMini {
    /// グループID
    pub id: GroupId,

    /// グループ名
    pub name: Option<String>,

    /// グループの種類
    pub group_type: Option<GroupType>,
}

/// グループの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GroupType {
    /// 管理者が管理するグループ
    ManagedGroup,

    /// エンタープライズのすべてのユーザーを含むグループ
    AllUsersGroup,

    /// BOXクライアントが知らない種類
    ///
    /// BOXが種類を追加しても、コラボレーション情報の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

/// アプリ項目
///
/// Box Hubsなどのアプリケーションが所有する項目である。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct AppItem {
    /// アプリ項目ID
    pub id: AppItemId,

    /// アプリ項目を所有するアプリケーションの種類（例: `hubs`）
    pub application_type: Option<String>,
}

/// コラボレーションを承認する前に満たす必要がある条件
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct AcceptanceRequirementsStatus {
    /// 利用規約への同意の条件
    pub terms_of_service_requirement: Option<TermsOfServiceRequirement>,

    /// 強力なパスワードの条件
    pub strong_password_requirement: Option<StrongPasswordRequirement>,

    /// 2要素認証の条件
    pub two_factor_authentication_requirement: Option<TwoFactorAuthenticationRequirement>,
}

/// 利用規約への同意の条件
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct TermsOfServiceRequirement {
    /// ユーザーが利用規約に同意したかどうか
    ///
    /// 利用規約への同意を求めていない場合は`None`となる。
    pub is_accepted: Option<bool>,

    /// 同意を求める利用規約
    ///
    /// 利用規約への同意を求めていない場合は`None`となる。
    pub terms_of_service: Option<TermsOfService>,
}

/// 利用規約
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct TermsOfService {
    /// 利用規約ID
    pub id: TermsOfServiceId,
}

/// 強力なパスワードの条件
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct StrongPasswordRequirement {
    /// コラボレーションを所有するエンタープライズが、外部のコラボレーターに強力なパスワードを求めているかどうか
    pub enterprise_has_strong_password_required_for_external_users: Option<bool>,

    /// ユーザーが強力なパスワードを設定しているかどうか
    ///
    /// 強力なパスワードを求めていない場合は`None`となる。
    pub user_has_strong_password: Option<bool>,
}

/// 2要素認証の条件
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct TwoFactorAuthenticationRequirement {
    /// コラボレーションを所有するエンタープライズが、2要素認証を求めているかどうか
    pub enterprise_has_two_factor_auth_enabled: Option<bool>,

    /// ユーザーが2要素認証を有効にしているかどうか
    ///
    /// 2要素認証を求めていない場合は`None`となる。
    pub user_has_two_factor_authentication_enabled: Option<bool>,
}

/// フォルダのコラボレーションをリストするときのオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// # Examples
///
/// ```
/// use box_client::ListFolderCollaborationsOptions;
///
/// let options = ListFolderCollaborationsOptions::default().with_limit(100);
/// assert_eq!(options.limit, Some(100));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListFolderCollaborationsOptions {
    /// 1ページで取得するコラボレーション数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,
}

impl ListFolderCollaborationsOptions {
    /// 1ページで取得するコラボレーション数の上限を設定する。
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。範囲外の値は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// ファイルのコラボレーションをリストするときのオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// # Examples
///
/// ```
/// use box_client::ListFileCollaborationsOptions;
///
/// let options = ListFileCollaborationsOptions::default().with_limit(100);
/// assert_eq!(options.limit, Some(100));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListFileCollaborationsOptions {
    /// 1ページで取得するコラボレーション数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,
}

impl ListFileCollaborationsOptions {
    /// 1ページで取得するコラボレーション数の上限を設定する。
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。範囲外の値は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// フォルダまたはファイルのコラボレーションの1ページ
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct CollaborationsPage {
    /// このページのコラボレーション
    pub entries: Vec<Collaboration>,

    /// このページの取得に使用したコラボレーション数の上限
    pub limit: u32,

    /// 次のページを取得するためのマーカー
    ///
    /// 後続のコラボレーションがない場合は`None`になる。
    pub next_marker: Option<String>,
}

impl MarkerPage for CollaborationsPage {
    type Entry = Collaboration;

    fn into_parts(self) -> (Vec<Collaboration>, Option<String>) {
        (self.entries, self.next_marker)
    }
}

impl BoxClient {
    /// コラボレーション情報を取得する。
    ///
    /// # Arguments
    ///
    /// - `collaboration_id`: 情報を取得するコラボレーションのID
    ///
    /// # Errors
    ///
    /// コラボレーションが存在しない場合、またはコラボレーションにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn collaboration_information(
        &self,
        collaboration_id: &CollaborationId,
    ) -> BoxClientResult<Collaboration> {
        self.send_json(ApiRequest::get([
            "collaborations",
            collaboration_id.as_ref(),
        ]))
        .await
    }

    /// フォルダのコラボレーションを1ページ分リストする。
    ///
    /// BOXは、承認済みのコラボレーションに加えて、招待を送信して承認を待っている
    /// コラボレーション（状態が[`CollaborationStatus::Pending`]）も返す。
    /// フォルダにコラボレーションがない場合は、空のページを返す。
    ///
    /// 1回の呼び出しで返すコラボレーションは最大で`limit`件であるため、後続のコラボレーションが残っている
    /// 場合がある。後続のコラボレーションがある場合は、戻り値の[`CollaborationsPage::next_marker`]が`Some`になる。
    /// そのマーカーを`marker`に指定して再度呼び出すと、次のページを取得できる。
    /// すべてのコラボレーションが必要な場合は、[`BoxClient::list_all_folder_collaborations`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: コラボレーションをリストするフォルダのID
    /// - `marker`: 前のページの[`CollaborationsPage::next_marker`]。最初のページを取得する場合は`None`
    /// - `options`: コラボレーション数の上限などのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// フォルダが存在しない場合、またはフォルダにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_folder_collaborations(
        &self,
        folder_id: &FolderId,
        marker: Option<&str>,
        options: &ListFolderCollaborationsOptions,
    ) -> BoxClientResult<CollaborationsPage> {
        let request = marker_page_request(
            ["folders", folder_id.as_ref(), "collaborations"],
            marker,
            options.limit,
            None,
            None,
        )?;

        self.send_json(request).await
    }

    /// フォルダのすべてのコラボレーションを、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: コラボレーションをリストするフォルダのID
    /// - `options`: コラボレーション数の上限などのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// BOXが以前に返したマーカーを再び返した場合、このメソッドは無限ループを避けるため
    /// [`BoxClientError::UnexpectedResponse`](crate::BoxClientError::UnexpectedResponse)を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みのコラボレーションを破棄してエラーを返す。
    pub async fn list_all_folder_collaborations(
        &self,
        folder_id: &FolderId,
        options: &ListFolderCollaborationsOptions,
    ) -> BoxClientResult<Vec<Collaboration>> {
        let options = ListFolderCollaborationsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
        };
        let (collaborations, pages) = collect_all_pages(async |marker| {
            self.list_folder_collaborations(folder_id, marker, &options)
                .await
        })
        .await?;

        tracing::debug!(
            folder_id = folder_id.as_ref(),
            pages,
            collaborations = collaborations.len(),
            "listed all collaborations of folder"
        );

        Ok(collaborations)
    }

    /// ファイルのコラボレーションを1ページ分リストする。
    ///
    /// BOXは、承認済みのコラボレーションに加えて、招待を送信して承認を待っている
    /// コラボレーション（状態が[`CollaborationStatus::Pending`]）も返す。
    /// ファイルにコラボレーションがない場合は、空のページを返す。
    ///
    /// 1回の呼び出しで返すコラボレーションは最大で`limit`件であるため、後続のコラボレーションが残っている
    /// 場合がある。後続のコラボレーションがある場合は、戻り値の[`CollaborationsPage::next_marker`]が`Some`になる。
    /// そのマーカーを`marker`に指定して再度呼び出すと、次のページを取得できる。
    /// すべてのコラボレーションが必要な場合は、[`BoxClient::list_all_file_collaborations`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: コラボレーションをリストするファイルのID
    /// - `marker`: 前のページの[`CollaborationsPage::next_marker`]。最初のページを取得する場合は`None`
    /// - `options`: コラボレーション数の上限などのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// ファイルが存在しない場合、またはファイルにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_file_collaborations(
        &self,
        file_id: &FileId,
        marker: Option<&str>,
        options: &ListFileCollaborationsOptions,
    ) -> BoxClientResult<CollaborationsPage> {
        let request = marker_page_request(
            ["files", file_id.as_ref(), "collaborations"],
            marker,
            options.limit,
            None,
            None,
        )?;

        self.send_json(request).await
    }

    /// ファイルのすべてのコラボレーションを、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: コラボレーションをリストするファイルのID
    /// - `options`: コラボレーション数の上限などのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// BOXが以前に返したマーカーを再び返した場合、このメソッドは無限ループを避けるため
    /// [`BoxClientError::UnexpectedResponse`](crate::BoxClientError::UnexpectedResponse)を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みのコラボレーションを破棄してエラーを返す。
    pub async fn list_all_file_collaborations(
        &self,
        file_id: &FileId,
        options: &ListFileCollaborationsOptions,
    ) -> BoxClientResult<Vec<Collaboration>> {
        let options = ListFileCollaborationsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
        };
        let (collaborations, pages) = collect_all_pages(async |marker| {
            self.list_file_collaborations(file_id, marker, &options)
                .await
        })
        .await?;

        tracing::debug!(
            file_id = file_id.as_ref(),
            pages,
            collaborations = collaborations.len(),
            "listed all collaborations of file"
        );

        Ok(collaborations)
    }
}

#[cfg(test)]
mod test_collaboration_information {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        BoxClientError, FolderId, FolderItem,
        test_support::{create_client, mount_token, not_found_response, short_retry},
    };

    /// `GET /collaborations/{collaboration_id}`を呼び出し、コラボレーション情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_collaboration_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/collaborations/12345678"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "collaboration",
                "id": "12345678",
                "item": {
                    "type": "folder",
                    "id": "12345",
                    "etag": "1",
                    "sequence_id": "3",
                    "name": "Contracts",
                },
                "app_item": null,
                "accessible_by": {
                    "type": "user",
                    "id": "11446498",
                    "name": "Aaron Levie",
                    "login": "ceo@example.com",
                    "is_active": true,
                },
                "invite_email": null,
                "role": "editor",
                "expires_at": "2012-12-26T10:53:43-08:00",
                "is_access_only": true,
                "status": "accepted",
                "acknowledged_at": "2012-12-12T10:55:20-08:00",
                "created_by": {
                    "type": "user",
                    "id": "33224412",
                    "name": "Dylan Smith",
                    "login": "dylan@example.com",
                    "is_active": true,
                },
                "created_at": "2012-12-12T10:53:43-08:00",
                "modified_at": "2012-12-12T10:53:43-08:00",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let collaboration = client
            .collaboration_information(&CollaborationId::new("12345678"))
            .await?;

        assert_eq!(collaboration.id, CollaborationId::new("12345678"));
        assert!(
            matches!(
                &collaboration.item,
                Some(BoxItem::Folder(FolderItem { id, name, .. }))
                    if *id == FolderId::new("12345") && name == "Contracts"
            ),
            "item: {:?}",
            collaboration.item
        );
        assert_eq!(collaboration.app_item, None);
        assert!(
            matches!(
                &collaboration.accessible_by,
                Some(CollaborationGrantee::User(user))
                    if user.id == UserId::new("11446498")
                        && user.login.as_deref() == Some("ceo@example.com")
                        && user.is_active == Some(true)
            ),
            "accessible_by: {:?}",
            collaboration.accessible_by
        );
        assert_eq!(collaboration.role, Some(CollaborationRole::Editor));
        assert_eq!(collaboration.status, Some(CollaborationStatus::Accepted));
        assert_eq!(collaboration.is_access_only, Some(true));
        assert_eq!(
            collaboration.expires_at,
            Some(time::macros::datetime!(2012-12-26 10:53:43 -08:00))
        );
        assert_eq!(
            collaboration.created_by.map(|user| user.id),
            Some(UserId::new("33224412"))
        );

        Ok(())
    }

    /// コラボレーションを承認する前に満たす必要がある条件をデシリアライズすることを確認
    #[test]
    fn test_acceptance_requirements_status() -> anyhow::Result<()> {
        let requirements: AcceptanceRequirementsStatus = serde_json::from_value(json!({
            "terms_of_service_requirement": {
                "is_accepted": true,
                "terms_of_service": {"type": "terms_of_service", "id": "11446498"},
            },
            "strong_password_requirement": {
                "enterprise_has_strong_password_required_for_external_users": true,
                "user_has_strong_password": true,
            },
            "two_factor_authentication_requirement": {
                "enterprise_has_two_factor_auth_enabled": true,
                "user_has_two_factor_authentication_enabled": null,
            },
        }))?;

        assert_eq!(
            requirements
                .terms_of_service_requirement
                .and_then(|r| r.terms_of_service)
                .map(|t| t.id),
            Some(TermsOfServiceId::new("11446498"))
        );
        assert_eq!(
            requirements
                .strong_password_requirement
                .and_then(|r| r.user_has_strong_password),
            Some(true)
        );
        // 2要素認証を求めていない場合は、BOXがnullを返す。
        assert_eq!(
            requirements.two_factor_authentication_requirement,
            Some(TwoFactorAuthenticationRequirement {
                enterprise_has_two_factor_auth_enabled: Some(true),
                user_has_two_factor_authentication_enabled: None,
            })
        );

        Ok(())
    }

    /// コラボレーションが存在しない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_collaboration_information_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/collaborations/999"))
            .respond_with(not_found_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let result = client
            .collaboration_information(&CollaborationId::new("999"))
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_list_folder_collaborations {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        BoxClientError,
        test_support::{
            create_client, mount_token, not_found_response, request_queries, short_retry,
        },
    };

    /// フォルダのコラボレーションのAPIのパス
    const COLLABORATIONS_PATH: &str = "/2.0/folders/123/collaborations";

    /// マーカーとオプションをクエリパラメーターに変換し、ページを返すことを確認
    #[rstest]
    #[case::default(None, ListFolderCollaborationsOptions::default(), "usemarker=true")]
    #[case::all(
        Some("m1"),
        ListFolderCollaborationsOptions::default().with_limit(1000),
        "usemarker=true&limit=1000&marker=m1"
    )]
    #[tokio::test]
    async fn test_list_folder_collaborations(
        #[case] marker: Option<&str>,
        #[case] options: ListFolderCollaborationsOptions,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [
                    {"type": "collaboration", "id": "1", "role": "editor", "status": "accepted"},
                    {"type": "collaboration", "id": "2", "item": null, "status": "pending"},
                ],
                "limit": 100,
                "next_marker": "m2",
                "prev_marker": null,
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client
            .list_folder_collaborations(&FolderId::new("123"), marker, &options)
            .await?;

        let ids: Vec<_> = page.entries.iter().map(|c| c.id.as_ref()).collect();
        assert_eq!(ids, vec!["1", "2"]);
        assert_eq!(page.entries[1].status, Some(CollaborationStatus::Pending));
        assert_eq!(page.limit, 100);
        assert_eq!(page.next_marker.as_deref(), Some("m2"));
        assert_eq!(
            request_queries(&server, COLLABORATIONS_PATH).await,
            vec![expected_query]
        );

        Ok(())
    }

    /// コラボレーションがないフォルダでは、空のページを返すことを確認
    #[tokio::test]
    async fn test_list_folder_collaborations_empty() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [],
                "limit": 100,
                "next_marker": null,
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client
            .list_folder_collaborations(
                &FolderId::new("123"),
                None,
                &ListFolderCollaborationsOptions::default(),
            )
            .await?;

        assert_eq!(page.entries, Vec::new());
        assert_eq!(page.next_marker, None);

        Ok(())
    }

    /// コラボレーション数の上限が範囲外の場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::zero(0)]
    #[case::over_max(MAX_ITEMS_LIMIT + 1)]
    #[tokio::test]
    async fn test_list_folder_collaborations_invalid_limit(
        #[case] limit: u32,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;

        let result = client
            .list_folder_collaborations(
                &FolderId::new("123"),
                None,
                &ListFolderCollaborationsOptions::default().with_limit(limit),
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(
            request_queries(&server, COLLABORATIONS_PATH).await,
            Vec::<String>::new()
        );

        Ok(())
    }

    /// フォルダが存在しない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_list_folder_collaborations_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .respond_with(not_found_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let result = client
            .list_folder_collaborations(
                &FolderId::new("123"),
                None,
                &ListFolderCollaborationsOptions::default(),
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_list_all_folder_collaborations {
    use rstest::rstest;
    use serde_json::{Value, json};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    use super::*;
    use crate::{
        BoxClientError,
        test_support::{create_client, mount_token, short_retry},
    };

    /// フォルダのコラボレーションのAPIのパス
    const COLLABORATIONS_PATH: &str = "/2.0/folders/123/collaborations";

    /// 指定したIDのコラボレーションだけを含むページのレスポンスを返す。
    fn page_response(id: &str, next_marker: &Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "entries": [{"type": "collaboration", "id": id}],
            "limit": 1000,
            "next_marker": next_marker,
        }))
    }

    /// `next_marker`がnullまたは空文字列になるまでページを送り、すべてのコラボレーションを返すことを確認
    #[rstest]
    #[case::null(Value::Null)]
    #[case::empty(json!(""))]
    #[tokio::test]
    async fn test_list_all_folder_collaborations(#[case] last_marker: Value) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        // コラボレーション数の上限を省略した場合は、上限の1000件ずつ取得する。
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param_is_missing("marker"))
            .respond_with(page_response("1", &json!("m1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param("marker", "m1"))
            .respond_with(page_response("2", &last_marker))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let collaborations = client
            .list_all_folder_collaborations(
                &FolderId::new("123"),
                &ListFolderCollaborationsOptions::default(),
            )
            .await?;

        let ids: Vec<_> = collaborations.iter().map(|c| c.id.as_ref()).collect();
        assert_eq!(ids, vec!["1", "2"]);

        Ok(())
    }

    /// コラボレーション数の上限を指定した場合は、その上限で各ページを取得することを確認
    #[tokio::test]
    async fn test_list_all_folder_collaborations_with_limit() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .and(query_param("limit", "10"))
            .respond_with(page_response("1", &Value::Null))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let collaborations = client
            .list_all_folder_collaborations(
                &FolderId::new("123"),
                &ListFolderCollaborationsOptions::default().with_limit(10),
            )
            .await?;

        assert_eq!(collaborations.len(), 1);

        Ok(())
    }

    /// BOXが同じマーカーを続けて返す場合は、無限ループせずにエラーを返すことを確認
    #[tokio::test]
    async fn test_list_all_folder_collaborations_marker_not_advanced() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .respond_with(page_response("1", &json!("m1")))
            .expect(2)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let result = client
            .list_all_folder_collaborations(
                &FolderId::new("123"),
                &ListFolderCollaborationsOptions::default(),
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_list_file_collaborations {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        BoxClientError,
        test_support::{
            create_client, mount_token, not_found_response, request_queries, short_retry,
        },
    };

    /// ファイルのコラボレーションのAPIのパス
    const COLLABORATIONS_PATH: &str = "/2.0/files/456/collaborations";

    /// マーカーとオプションをクエリパラメーターに変換し、ページを返すことを確認
    #[rstest]
    #[case::default(None, ListFileCollaborationsOptions::default(), "usemarker=true")]
    #[case::all(
        Some("m1"),
        ListFileCollaborationsOptions::default().with_limit(1000),
        "usemarker=true&limit=1000&marker=m1"
    )]
    #[tokio::test]
    async fn test_list_file_collaborations(
        #[case] marker: Option<&str>,
        #[case] options: ListFileCollaborationsOptions,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [
                    {
                        "type": "collaboration",
                        "id": "1",
                        "item": {"type": "file", "id": "456", "etag": "0", "name": "Contract.pdf"},
                        "role": "viewer",
                        "status": "accepted",
                    },
                    {"type": "collaboration", "id": "2", "item": null, "status": "pending"},
                ],
                "limit": 100,
                "next_marker": "m2",
                "prev_marker": null,
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client
            .list_file_collaborations(&FileId::new("456"), marker, &options)
            .await?;

        let ids: Vec<_> = page.entries.iter().map(|c| c.id.as_ref()).collect();
        assert_eq!(ids, vec!["1", "2"]);
        assert!(
            matches!(
                &page.entries[0].item,
                Some(BoxItem::File(file)) if file.id == FileId::new("456")
            ),
            "item: {:?}",
            page.entries[0].item
        );
        assert_eq!(page.entries[1].status, Some(CollaborationStatus::Pending));
        assert_eq!(page.next_marker.as_deref(), Some("m2"));
        assert_eq!(
            request_queries(&server, COLLABORATIONS_PATH).await,
            vec![expected_query]
        );

        Ok(())
    }

    /// コラボレーション数の上限が範囲外の場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::zero(0)]
    #[case::over_max(MAX_ITEMS_LIMIT + 1)]
    #[tokio::test]
    async fn test_list_file_collaborations_invalid_limit(#[case] limit: u32) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;

        let result = client
            .list_file_collaborations(
                &FileId::new("456"),
                None,
                &ListFileCollaborationsOptions::default().with_limit(limit),
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(
            request_queries(&server, COLLABORATIONS_PATH).await,
            Vec::<String>::new()
        );

        Ok(())
    }

    /// ファイルが存在しない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_list_file_collaborations_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .respond_with(not_found_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let result = client
            .list_file_collaborations(
                &FileId::new("456"),
                None,
                &ListFileCollaborationsOptions::default(),
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_list_all_file_collaborations {
    use serde_json::{Value, json};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// ファイルのコラボレーションのAPIのパス
    const COLLABORATIONS_PATH: &str = "/2.0/files/456/collaborations";

    /// 指定したIDのコラボレーションだけを含むページのレスポンスを返す。
    fn page_response(id: &str, next_marker: &Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "entries": [{"type": "collaboration", "id": id}],
            "limit": 1000,
            "next_marker": next_marker,
        }))
    }

    /// `next_marker`がnullになるまでページを送り、すべてのコラボレーションを返すことを確認
    ///
    /// マーカーの終端と重複の扱いは、フォルダのコラボレーションのテストと`collect_all_pages`のテストで確認する。
    #[tokio::test]
    async fn test_list_all_file_collaborations() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        // コラボレーション数の上限を省略した場合は、上限の1000件ずつ取得する。
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param_is_missing("marker"))
            .respond_with(page_response("1", &json!("m1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param("marker", "m1"))
            .respond_with(page_response("2", &Value::Null))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let collaborations = client
            .list_all_file_collaborations(
                &FileId::new("456"),
                &ListFileCollaborationsOptions::default(),
            )
            .await?;

        let ids: Vec<_> = collaborations.iter().map(|c| c.id.as_ref()).collect();
        assert_eq!(ids, vec!["1", "2"]);

        Ok(())
    }

    /// コラボレーション数の上限を指定した場合は、その上限で各ページを取得することを確認
    #[tokio::test]
    async fn test_list_all_file_collaborations_with_limit() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .and(query_param("limit", "10"))
            .respond_with(page_response("1", &Value::Null))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let collaborations = client
            .list_all_file_collaborations(
                &FileId::new("456"),
                &ListFileCollaborationsOptions::default().with_limit(10),
            )
            .await?;

        assert_eq!(collaborations.len(), 1);

        Ok(())
    }
}

#[cfg(test)]
mod test_deserialize_collaboration {
    use rstest::rstest;
    use serde_json::json;

    use super::*;

    /// `id`と`type`だけのコラボレーションを、ほかの属性を`None`としてデシリアライズすることを確認
    #[test]
    fn test_minimal() -> anyhow::Result<()> {
        let collaboration: Collaboration =
            serde_json::from_value(json!({"type": "collaboration", "id": "1"}))?;

        assert_eq!(collaboration.id, CollaborationId::new("1"));
        assert_eq!(collaboration.item, None);
        assert_eq!(collaboration.accessible_by, None);
        assert_eq!(collaboration.role, None);
        assert_eq!(collaboration.status, None);
        assert_eq!(collaboration.expires_at, None);
        assert_eq!(collaboration.acceptance_requirements_status, None);

        Ok(())
    }

    /// 承認待ちのコラボレーションで、対象の項目がnull、表示名が空文字列の場合もデシリアライズすることを確認
    #[test]
    fn test_pending() -> anyhow::Result<()> {
        let collaboration: Collaboration = serde_json::from_value(json!({
            "type": "collaboration",
            "id": "1",
            "item": null,
            "accessible_by": {
                "type": "user",
                "id": "2",
                "name": "",
                "login": "invitee@example.com",
            },
            "invite_email": "invitee@example.com",
            "status": "pending",
            "expires_at": null,
        }))?;

        assert_eq!(collaboration.item, None);
        assert_eq!(collaboration.status, Some(CollaborationStatus::Pending));
        assert_eq!(
            collaboration.invite_email.as_deref(),
            Some("invitee@example.com")
        );
        assert!(
            matches!(
                &collaboration.accessible_by,
                Some(CollaborationGrantee::User(user))
                    if user.name.as_deref() == Some("") && user.is_active.is_none()
            ),
            "accessible_by: {:?}",
            collaboration.accessible_by
        );

        Ok(())
    }

    /// アクセス権限を与えた相手がグループの場合に、グループとしてデシリアライズすることを確認
    #[rstest]
    #[case::managed("managed_group", GroupType::ManagedGroup)]
    #[case::all_users("all_users_group", GroupType::AllUsersGroup)]
    #[case::unknown("new_group_type", GroupType::Unknown)]
    fn test_group_grantee(
        #[case] group_type: &str,
        #[case] expected: GroupType,
    ) -> anyhow::Result<()> {
        let grantee: CollaborationGrantee = serde_json::from_value(json!({
            "type": "group",
            "id": "3",
            "name": "Support",
            "group_type": group_type,
        }))?;

        let CollaborationGrantee::Group(group) = grantee else {
            anyhow::bail!("unexpected grantee: {grantee:?}");
        };
        assert_eq!(group.id, GroupId::new("3"));
        assert_eq!(group.name.as_deref(), Some("Support"));
        assert_eq!(group.group_type, Some(expected));

        Ok(())
    }

    /// アクセス権限を与えた相手の種類が未知の場合は、`Unknown`としてデシリアライズすることを確認
    #[test]
    fn test_unknown_grantee() -> anyhow::Result<()> {
        let grantee: CollaborationGrantee =
            serde_json::from_value(json!({"type": "robot", "id": "4"}))?;

        assert_eq!(grantee, CollaborationGrantee::Unknown);

        Ok(())
    }

    /// 対象がアプリ項目の場合に、アプリ項目としてデシリアライズすることを確認
    #[test]
    fn test_app_item() -> anyhow::Result<()> {
        let collaboration: Collaboration = serde_json::from_value(json!({
            "type": "collaboration",
            "id": "1",
            "item": null,
            "app_item": {"type": "app_item", "id": "5", "application_type": "hubs"},
        }))?;

        assert_eq!(collaboration.item, None);
        let app_item = collaboration
            .app_item
            .ok_or_else(|| anyhow::anyhow!("app_item is missing"))?;
        assert_eq!(app_item.id, AppItemId::new("5"));
        assert_eq!(app_item.application_type.as_deref(), Some("hubs"));

        Ok(())
    }

    /// BOXが返す役割の文字列を、対応する役割としてデシリアライズすることを確認
    #[rstest]
    #[case::editor("editor", CollaborationRole::Editor)]
    #[case::viewer("viewer", CollaborationRole::Viewer)]
    #[case::previewer("previewer", CollaborationRole::Previewer)]
    #[case::uploader("uploader", CollaborationRole::Uploader)]
    #[case::previewer_uploader("previewer uploader", CollaborationRole::PreviewerUploader)]
    #[case::viewer_uploader("viewer uploader", CollaborationRole::ViewerUploader)]
    #[case::co_owner("co-owner", CollaborationRole::CoOwner)]
    #[case::owner("owner", CollaborationRole::Owner)]
    #[case::unknown("new role", CollaborationRole::Unknown)]
    fn test_role(#[case] value: &str, #[case] expected: CollaborationRole) -> anyhow::Result<()> {
        let role: CollaborationRole = serde_json::from_value(json!(value))?;

        assert_eq!(role, expected);

        Ok(())
    }

    /// BOXが返す状態の文字列を、対応する状態としてデシリアライズすることを確認
    #[rstest]
    #[case::accepted("accepted", CollaborationStatus::Accepted)]
    #[case::pending("pending", CollaborationStatus::Pending)]
    #[case::rejected("rejected", CollaborationStatus::Rejected)]
    #[case::unknown("expired", CollaborationStatus::Unknown)]
    fn test_status(
        #[case] value: &str,
        #[case] expected: CollaborationStatus,
    ) -> anyhow::Result<()> {
        let status: CollaborationStatus = serde_json::from_value(json!(value))?;

        assert_eq!(status, expected);

        Ok(())
    }
}
