//! グループに関するAPI
//!
//! グループは、エンタープライズのユーザーの集まりである。コラボレーションなどで、ユーザーの代わりにグループを指定できる。
//! グループの作成および削除は、管理者の権限を持つユーザーまたはアプリケーションだけが実行できる。

use garde::Validate;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, GroupId,
    api::{
        pagination::{
            MAX_ITEMS_LIMIT, MAX_OFFSET, OffsetPage, collect_offset_pages_up_to,
            offset_page_request, truncate_to_max_entries, validate_listed_offset,
        },
        types::validated,
    },
    request::ApiRequest,
    retry::Idempotency,
};

/// グループの出所の最大文字数
///
/// BOXのAPI仕様（OpenAPI）は、グループの作成および更新で、出所を255文字以下に制限している。
pub const MAX_GROUP_PROVENANCE_CHARS: usize = 255;

/// グループの説明の最大文字数
///
/// BOXのAPI仕様（OpenAPI）は、グループの作成および更新で、説明を255文字以下に制限している。
pub const MAX_GROUP_DESCRIPTION_CHARS: usize = 255;

/// エンタープライズのグループをリストするときに取得できるグループ数の上限
///
/// BOXは、エンタープライズのグループのリストをオフセット方式でだけページ送りし、オフセットが10,000を超えるリクエストを
/// 400で拒否する。そのため、BOXクライアントは、条件に一致するグループのうち、先頭からこの件数までしか返さない。
pub const MAX_LISTED_GROUPS: u64 = MAX_OFFSET;

/// BOXのグループ
///
/// BOXのAPI仕様（OpenAPI）は、`id`と`type`だけを必須としている。
/// BOXが属性を省略したりnullを返したりしても取得を失敗させないように、`id`以外の属性を`Option`で受け取る。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Group {
    /// グループID
    pub id: GroupId,

    /// グループ名
    pub name: Option<String>,

    /// グループの種類
    pub group_type: Option<GroupType>,

    /// 作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,

    /// 最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,

    /// グループの出所
    ///
    /// グループを同期する外部のサービス（例えば`Active Directory`）を表す。
    pub provenance: Option<String>,

    /// 外部のグループと関連付ける識別子
    ///
    /// 例えば、Active DirectoryのオブジェクトIDやGoogleグループのIDである。
    pub external_sync_identifier: Option<String>,

    /// グループの説明
    pub description: Option<String>,

    /// コラボレーションにグループを招待できるユーザーの範囲
    pub invitability_level: Option<GroupAccessLevel>,

    /// グループのメンバーを参照できるユーザーの範囲
    pub member_viewability_level: Option<GroupAccessLevel>,

    /// 認証中のユーザーがグループに対して持つ権限
    pub permissions: Option<GroupPermissions>,
}

/// 最小限のグループ情報
///
/// コラボレーションのアクセス権限を与えた相手などとして、ほかのリソースに含まれる。
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
    /// BOXが種類を追加しても、グループを含む情報の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

/// グループを操作できるユーザーの範囲
///
/// コラボレーションにグループを招待できるユーザーの範囲（[`Group::invitability_level`]）と、
/// グループのメンバーを参照できるユーザーの範囲（[`Group::member_viewability_level`]）で共有する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GroupAccessLevel {
    /// エンタープライズの管理者、共同管理者およびグループの管理者
    AdminsOnly,

    /// 管理者とグループのメンバー
    AdminsAndMembers,

    /// エンタープライズのすべての管理対象ユーザー
    AllManagedUsers,

    /// BOXクライアントが知らない範囲
    ///
    /// BOXが範囲を追加しても、グループ情報の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

impl GroupAccessLevel {
    /// リクエストボディに指定する文字列を返す。
    ///
    /// [`GroupAccessLevel::Unknown`]は、BOXに送信する値を持たないため、`None`を返す。
    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::AdminsOnly => Some("admins_only"),
            Self::AdminsAndMembers => Some("admins_and_members"),
            Self::AllManagedUsers => Some("all_managed_users"),
            Self::Unknown => None,
        }
    }
}

/// 操作できるユーザーの範囲をリクエストボディの値に変換する。
///
/// # Errors
///
/// `level`が[`GroupAccessLevel::Unknown`]の場合は、`message`の[`BoxClientError::InvalidArgument`]を返す。
fn access_level_str(
    level: Option<GroupAccessLevel>,
    message: &'static str,
) -> BoxClientResult<Option<&'static str>> {
    level
        .map(|level| {
            level
                .as_str()
                .ok_or(BoxClientError::InvalidArgument(message))
        })
        .transpose()
}

/// 認証中のユーザーがグループに対して持つ権限
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct GroupPermissions {
    /// コラボレーションにグループを招待できるかどうか
    pub can_invite_as_collaborator: Option<bool>,
}

/// グループの作成のオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// `None`の属性は送信せず、BOXの既定値を使用する。
///
/// # Examples
///
/// ```
/// use box_client::{CreateGroupOptions, GroupAccessLevel};
///
/// let options = CreateGroupOptions::default()
///     .with_description("サポート担当")
///     .with_invitability_level(GroupAccessLevel::AdminsAndMembers);
/// assert_eq!(options.description, Some("サポート担当"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CreateGroupOptions<'a> {
    /// グループの出所
    ///
    /// [`MAX_GROUP_PROVENANCE_CHARS`]文字以下で指定する。
    /// BOXのドキュメントによると、出所を設定すると、管理者はBOXのWebアプリでグループ名とメンバーを編集できなくなる。
    pub provenance: Option<&'a str>,

    /// 外部のグループと関連付ける識別子
    pub external_sync_identifier: Option<&'a str>,

    /// グループの説明
    ///
    /// [`MAX_GROUP_DESCRIPTION_CHARS`]文字以下で指定する。
    pub description: Option<&'a str>,

    /// コラボレーションにグループを招待できるユーザーの範囲
    ///
    /// [`GroupAccessLevel::Unknown`]は指定できない。
    pub invitability_level: Option<GroupAccessLevel>,

    /// グループのメンバーを参照できるユーザーの範囲
    ///
    /// [`GroupAccessLevel::Unknown`]は指定できない。
    pub member_viewability_level: Option<GroupAccessLevel>,
}

impl<'a> CreateGroupOptions<'a> {
    /// グループの出所を設定する。
    #[must_use]
    pub fn with_provenance(mut self, provenance: &'a str) -> Self {
        self.provenance = Some(provenance);
        self
    }

    /// 外部のグループと関連付ける識別子を設定する。
    #[must_use]
    pub fn with_external_sync_identifier(mut self, external_sync_identifier: &'a str) -> Self {
        self.external_sync_identifier = Some(external_sync_identifier);
        self
    }

    /// グループの説明を設定する。
    #[must_use]
    pub fn with_description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }

    /// コラボレーションにグループを招待できるユーザーの範囲を設定する。
    #[must_use]
    pub fn with_invitability_level(mut self, invitability_level: GroupAccessLevel) -> Self {
        self.invitability_level = Some(invitability_level);
        self
    }

    /// グループのメンバーを参照できるユーザーの範囲を設定する。
    #[must_use]
    pub fn with_member_viewability_level(
        mut self,
        member_viewability_level: GroupAccessLevel,
    ) -> Self {
        self.member_viewability_level = Some(member_viewability_level);
        self
    }
}

/// グループの更新で変更する属性
///
/// `None`の属性は送信せず、BOXは変更しない。
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// # Examples
///
/// ```
/// use box_client::{GroupAccessLevel, GroupUpdate};
///
/// let update = GroupUpdate::default()
///     .with_name("Customer Support")
///     .with_member_viewability_level(GroupAccessLevel::AdminsOnly);
/// assert_eq!(update.name, Some("Customer Support"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct GroupUpdate<'a> {
    /// グループ名
    ///
    /// 1文字以上で指定する。BOXのドキュメントによると、グループ名はエンタープライズで一意でなければならない。
    pub name: Option<&'a str>,

    /// グループの出所
    ///
    /// [`MAX_GROUP_PROVENANCE_CHARS`]文字以下で指定する。
    pub provenance: Option<&'a str>,

    /// 外部のグループと関連付ける識別子
    pub external_sync_identifier: Option<&'a str>,

    /// グループの説明
    ///
    /// [`MAX_GROUP_DESCRIPTION_CHARS`]文字以下で指定する。
    pub description: Option<&'a str>,

    /// コラボレーションにグループを招待できるユーザーの範囲
    ///
    /// [`GroupAccessLevel::Unknown`]は指定できない。
    pub invitability_level: Option<GroupAccessLevel>,

    /// グループのメンバーを参照できるユーザーの範囲
    ///
    /// [`GroupAccessLevel::Unknown`]は指定できない。
    pub member_viewability_level: Option<GroupAccessLevel>,
}

impl<'a> GroupUpdate<'a> {
    /// グループ名を設定する。
    #[must_use]
    pub fn with_name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    /// グループの出所を設定する。
    #[must_use]
    pub fn with_provenance(mut self, provenance: &'a str) -> Self {
        self.provenance = Some(provenance);
        self
    }

    /// 外部のグループと関連付ける識別子を設定する。
    #[must_use]
    pub fn with_external_sync_identifier(mut self, external_sync_identifier: &'a str) -> Self {
        self.external_sync_identifier = Some(external_sync_identifier);
        self
    }

    /// グループの説明を設定する。
    #[must_use]
    pub fn with_description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }

    /// コラボレーションにグループを招待できるユーザーの範囲を設定する。
    #[must_use]
    pub fn with_invitability_level(mut self, invitability_level: GroupAccessLevel) -> Self {
        self.invitability_level = Some(invitability_level);
        self
    }

    /// グループのメンバーを参照できるユーザーの範囲を設定する。
    #[must_use]
    pub fn with_member_viewability_level(
        mut self,
        member_viewability_level: GroupAccessLevel,
    ) -> Self {
        self.member_viewability_level = Some(member_viewability_level);
        self
    }
}

/// グループの作成リクエストおよび更新リクエストのボディ
///
/// BOXは、グループの作成と更新で同じ属性を受け付けるため、作成と更新で共有する。
/// 作成ではグループ名を必ず送信し、更新では`None`の属性を送信しない。
/// `garde`は`None`の場合に`length`を検証しない。
#[derive(Debug, Serialize, Validate)]
struct GroupBody<'a> {
    /// グループ名
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(length(chars, min = 1))]
    name: Option<&'a str>,

    /// グループの出所
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(length(chars, max = MAX_GROUP_PROVENANCE_CHARS))]
    provenance: Option<&'a str>,

    /// 外部のグループと関連付ける識別子
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    external_sync_identifier: Option<&'a str>,

    /// グループの説明
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(length(chars, max = MAX_GROUP_DESCRIPTION_CHARS))]
    description: Option<&'a str>,

    /// コラボレーションにグループを招待できるユーザーの範囲
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    invitability_level: Option<&'static str>,

    /// グループのメンバーを参照できるユーザーの範囲
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    member_viewability_level: Option<&'static str>,
}

impl<'a> GroupBody<'a> {
    /// グループの作成リクエストのボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// 操作できるユーザーの範囲が[`GroupAccessLevel::Unknown`]の場合は、[`BoxClientError::InvalidArgument`]を返す。
    /// グループ名が空文字列の場合、または出所もしくは説明が長すぎる場合は、[`BoxClientError::InvalidRequestBody`]を返す。
    fn create(name: &'a str, options: &CreateGroupOptions<'a>) -> BoxClientResult<Self> {
        Self::new(
            Some(name),
            &GroupUpdate {
                name: None,
                provenance: options.provenance,
                external_sync_identifier: options.external_sync_identifier,
                description: options.description,
                invitability_level: options.invitability_level,
                member_viewability_level: options.member_viewability_level,
            },
        )
    }

    /// グループの更新リクエストのボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// [`GroupBody::create`]と同じ場合にエラーを返す。
    fn update(update: &GroupUpdate<'a>) -> BoxClientResult<Self> {
        Self::new(update.name, update)
    }

    /// グループ名と、グループ名以外の属性からボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// [`GroupBody::create`]と同じ場合にエラーを返す。
    fn new(name: Option<&'a str>, attributes: &GroupUpdate<'a>) -> BoxClientResult<Self> {
        validated(Self {
            name,
            provenance: attributes.provenance,
            external_sync_identifier: attributes.external_sync_identifier,
            description: attributes.description,
            invitability_level: access_level_str(
                attributes.invitability_level,
                "invitability level of a group must not be unknown",
            )?,
            member_viewability_level: access_level_str(
                attributes.member_viewability_level,
                "member viewability level of a group must not be unknown",
            )?,
        })
    }
}

/// エンタープライズのグループをリストするときのオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// BOXクライアントは、検索語の有無にかかわらず、条件に一致するグループのうち、先頭から[`MAX_LISTED_GROUPS`]件までしか返さない。
/// したがって、条件に一致するグループが[`MAX_LISTED_GROUPS`]件より多い場合は、検索語を指定して絞り込む。
/// 理由は[`BoxClient::list_enterprise_groups`]を参照する。
///
/// # Examples
///
/// ```
/// use box_client::ListEnterpriseGroupsOptions;
///
/// let options = ListEnterpriseGroupsOptions::default()
///     .with_filter_term("Engineering")
///     .with_limit(100);
/// assert_eq!(options.filter_term, Some("Engineering"));
/// assert_eq!(options.limit, Some(100));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListEnterpriseGroupsOptions<'a> {
    /// 検索語
    ///
    /// グループ名が検索語で始まるグループだけを返す。
    pub filter_term: Option<&'a str>,

    /// 1ページで取得するグループ数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,
}

impl<'a> ListEnterpriseGroupsOptions<'a> {
    /// 検索語を設定する。
    #[must_use]
    pub fn with_filter_term(mut self, filter_term: &'a str) -> Self {
        self.filter_term = Some(filter_term);
        self
    }

    /// 1ページで取得するグループ数の上限を設定する。
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。範囲外の値は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// エンタープライズのグループの1ページ
///
/// BOXは、エンタープライズのグループのリストをオフセット方式でページ送りする。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct GroupsPage {
    /// このページのグループ
    pub entries: Vec<Group>,

    /// 最後のグループのオフセットに1を加えた値
    pub total_count: u64,

    /// このページの取得に使用したグループ数の上限
    ///
    /// BOXがレスポンスに含めない場合は`None`になる。
    pub limit: Option<u32>,

    /// このページの最初のグループのオフセット（0始まり）
    ///
    /// BOXがレスポンスに含めない場合は`None`になる。
    pub offset: Option<u64>,
}

impl OffsetPage for GroupsPage {
    type Entry = Group;

    fn into_parts(self) -> (Vec<Group>, u64) {
        (self.entries, self.total_count)
    }
}

impl BoxClient {
    /// グループ情報を取得する。
    ///
    /// BOXのドキュメントによると、グループのメンバー、または管理者の権限を持つユーザーだけが実行できる。
    ///
    /// # Arguments
    ///
    /// - `group_id`: 情報を取得するグループのID
    ///
    /// # Errors
    ///
    /// グループが存在しない場合、またはグループを参照する権限がない場合は、
    /// [`BoxClientError::Api`]を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn group_information(&self, group_id: &GroupId) -> BoxClientResult<Group> {
        self.send_json(ApiRequest::get(["groups", group_id.as_ref()]))
            .await
    }

    /// エンタープライズのグループを1ページ分リストする。
    ///
    /// BOXのドキュメントによると、管理者の権限を持つユーザーだけが実行できる。グループがない場合は、空のページを返す。
    ///
    /// BOXは、このAPIをオフセット方式でだけページ送りし、マーカー方式に対応しない。
    /// また、BOXはオフセットが10,000を超えるリクエストを拒否し、それより後ろのグループを取得できない。
    /// そのため、BOXクライアントは、検索語の有無にかかわらず、条件に一致するグループのうち、
    /// 先頭から[`MAX_LISTED_GROUPS`]件までしか返さない。上限を超えるグループは、ページから取り除く。
    /// したがって、条件に一致するグループが[`MAX_LISTED_GROUPS`]件より多い場合は、検索語を指定して絞り込む。
    ///
    /// 1回の呼び出しで返すグループは最大で`limit`件である。次のページを取得する場合は、`offset`に、
    /// 前のページの`offset`と`entries`の数の和を指定して再度呼び出す。その和が[`MAX_LISTED_GROUPS`]に達した場合は、
    /// 後続のグループが残っていても、それ以上取得できない。
    /// すべてのグループが必要な場合は、[`BoxClient::list_all_enterprise_groups`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `offset`: 取得する最初のグループのオフセット（0始まり）。[`MAX_LISTED_GROUPS`]より小さい値を指定する。
    /// - `options`: 検索語およびグループ数の上限のオプション
    ///
    /// # Errors
    ///
    /// `offset`が[`MAX_LISTED_GROUPS`]以上の場合、または`options.limit`が範囲外の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// エンタープライズのグループを参照する権限がない場合は、[`BoxClientError::Api`]を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_enterprise_groups(
        &self,
        offset: u64,
        options: &ListEnterpriseGroupsOptions<'_>,
    ) -> BoxClientResult<GroupsPage> {
        let mut request =
            offset_page_request(["groups"], validate_listed_offset(offset)?, options.limit)?;
        if let Some(filter_term) = options.filter_term {
            request = request.query("filter_term", filter_term);
        }

        let mut page: GroupsPage = self.send_json(request).await?;
        // 先頭から上限の件数までしか返さないように、上限を超えるグループを取り除く。
        truncate_to_max_entries(&mut page.entries, offset, MAX_LISTED_GROUPS);
        Ok(page)
    }

    /// エンタープライズのすべてのグループを、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。
    /// 取得したグループの数が`total_count`もしくは[`MAX_LISTED_GROUPS`]に達した場合、
    /// またはBOXが空のページを返した場合に終了する。
    ///
    /// [`BoxClient::list_enterprise_groups`]と同じ理由で、検索語の有無にかかわらず、条件に一致するグループのうち、
    /// 先頭から[`MAX_LISTED_GROUPS`]件までしか返さない。条件に一致するグループがそれより多くても、エラーにはせず、
    /// 先頭から[`MAX_LISTED_GROUPS`]件を返す。したがって、条件に一致するグループが[`MAX_LISTED_GROUPS`]件より多い場合は、
    /// 検索語を指定して絞り込む。
    ///
    /// # Arguments
    ///
    /// - `options`: 検索語およびグループ数の上限のオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みのグループを破棄してエラーを返す。
    pub async fn list_all_enterprise_groups(
        &self,
        options: &ListEnterpriseGroupsOptions<'_>,
    ) -> BoxClientResult<Vec<Group>> {
        let options = ListEnterpriseGroupsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
            ..*options
        };
        let (groups, pages) = collect_offset_pages_up_to(MAX_LISTED_GROUPS, async |offset| {
            self.list_enterprise_groups(offset, &options).await
        })
        .await?;

        tracing::debug!(pages, groups = groups.len(), "listed all enterprise groups");

        Ok(groups)
    }

    /// エンタープライズにグループを作成する。
    ///
    /// BOXのドキュメントによると、管理者の権限を持つユーザーだけが実行できる。
    ///
    /// HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。
    /// 5xxまたはタイムアウトの後に再送すると、BOXがグループを作成済みの場合に、グループ名の重複で409を受け取るおそれがあるためである。
    ///
    /// # Arguments
    ///
    /// - `name`: グループ名。1文字以上で指定する。BOXのドキュメントによると、エンタープライズで一意でなければならない。
    /// - `options`: 出所、説明および操作できるユーザーの範囲などのオプション
    ///
    /// # Errors
    ///
    /// 操作できるユーザーの範囲が[`GroupAccessLevel::Unknown`]の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// グループ名が空文字列の場合、または出所もしくは説明が長すぎる場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// BOXのドキュメントによると、グループ名がエンタープライズで一意でない場合、BOXはエラーコードが`invalid_parameter`の409を返し、
    /// BOXクライアントは[`BoxClientError::Api`]を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがグループを作成した可能性がある。
    /// したがって、[`BoxClient::list_enterprise_groups`]でグループを確認する。
    pub async fn create_group(
        &self,
        name: &str,
        options: &CreateGroupOptions<'_>,
    ) -> BoxClientResult<Group> {
        let body = GroupBody::create(name, options)?;
        let request = ApiRequest::post(["groups"]).json(&body)?;

        let group: Group = self.send_json(request).await?;

        tracing::debug!(group_id = group.id.as_ref(), "created group");

        Ok(group)
    }

    /// グループを更新する。
    ///
    /// BOXのドキュメントによると、グループの管理者、または管理者の権限を持つユーザーだけが実行できる。
    ///
    /// 同じ値を何度設定しても結果が変わらないため、HTTPメソッドのとおり冪等なリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `group_id`: 更新するグループのID
    /// - `update`: 変更する属性
    ///
    /// # Errors
    ///
    /// 操作できるユーザーの範囲が[`GroupAccessLevel::Unknown`]の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// グループ名が空文字列の場合、または出所もしくは説明が長すぎる場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// BOXのドキュメントによると、グループ名がエンタープライズで一意でない場合、BOXはエラーコードが`invalid_parameter`の409を返し、
    /// BOXクライアントは[`BoxClientError::Api`]を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn update_group(
        &self,
        group_id: &GroupId,
        update: &GroupUpdate<'_>,
    ) -> BoxClientResult<Group> {
        let body = GroupBody::update(update)?;
        let request = ApiRequest::put(["groups", group_id.as_ref()]).json(&body)?;

        let group: Group = self.send_json(request).await?;

        tracing::debug!(group_id = group_id.as_ref(), "updated group");

        Ok(group)
    }

    /// グループを削除する。
    ///
    /// BOXのドキュメントによると、グループを完全に削除し、管理者の権限を持つユーザーだけが実行できる。
    ///
    /// BOXクライアントは、冪等なリクエストであれば、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがグループの削除を完了している可能性がある。
    /// BOXが削除した後にリクエストを再送すると、削除済みのグループに対する削除となるため、BOXはエラーを返しうる。
    /// その結果、削除に成功したにもかかわらず、削除に失敗したと誤って判断するおそれがある。
    /// したがって、HTTPメソッドがDELETEであっても冪等でないリクエストとして送信し、
    /// BOXがリクエストを処理していないことが明らかな429、エラーコードが`operation_blocked_temporary`の409
    /// および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `group_id`: 削除するグループのID
    ///
    /// # Errors
    ///
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがグループを削除した可能性があるため、
    /// [`BoxClient::group_information`]でグループが存在するか確認する。
    pub async fn delete_group(&self, group_id: &GroupId) -> BoxClientResult<()> {
        let request = ApiRequest::delete(["groups", group_id.as_ref()])
            .idempotency(Idempotency::NonIdempotent);

        self.send_empty(request).await?;

        tracing::debug!(group_id = group_id.as_ref(), "deleted group");

        Ok(())
    }
}

#[cfg(test)]
mod test_group {
    use rstest::rstest;
    use serde_json::json;
    use time::macros::datetime;

    use super::*;

    /// 完全な表現のすべての属性をデシリアライズすることを確認
    #[test]
    fn test_deserialize_group() -> anyhow::Result<()> {
        let group: Group = serde_json::from_value(json!({
            "id": "11446498",
            "type": "group",
            "name": "Support",
            "group_type": "managed_group",
            "created_at": "2012-12-12T10:53:43-08:00",
            "modified_at": "2012-12-12T10:53:43-08:00",
            "provenance": "Active Directory",
            "external_sync_identifier": "AD:123456",
            "description": "Support Group - as imported from Active Directory",
            "invitability_level": "admins_only",
            "member_viewability_level": "all_managed_users",
            "permissions": {"can_invite_as_collaborator": true},
        }))?;

        assert_eq!(group.id, GroupId::new("11446498"));
        assert_eq!(group.name.as_deref(), Some("Support"));
        assert_eq!(group.group_type, Some(GroupType::ManagedGroup));
        assert_eq!(
            group.created_at,
            Some(datetime!(2012-12-12 10:53:43 -08:00))
        );
        assert_eq!(
            group.modified_at,
            Some(datetime!(2012-12-12 10:53:43 -08:00))
        );
        assert_eq!(group.provenance.as_deref(), Some("Active Directory"));
        assert_eq!(group.external_sync_identifier.as_deref(), Some("AD:123456"));
        assert_eq!(
            group.description.as_deref(),
            Some("Support Group - as imported from Active Directory")
        );
        assert_eq!(group.invitability_level, Some(GroupAccessLevel::AdminsOnly));
        assert_eq!(
            group.member_viewability_level,
            Some(GroupAccessLevel::AllManagedUsers)
        );
        assert_eq!(
            group.permissions,
            Some(GroupPermissions {
                can_invite_as_collaborator: Some(true),
            })
        );

        Ok(())
    }

    /// `id`と`type`以外の属性が省略またはnullの場合も、`None`としてデシリアライズすることを確認
    #[rstest]
    #[case::missing(json!({"id": "1", "type": "group"}))]
    #[case::null(json!({
        "id": "1",
        "type": "group",
        "name": null,
        "group_type": null,
        "created_at": null,
        "modified_at": null,
        "provenance": null,
        "external_sync_identifier": null,
        "description": null,
        "invitability_level": null,
        "member_viewability_level": null,
        "permissions": null,
    }))]
    fn test_deserialize_group_without_attributes(
        #[case] value: serde_json::Value,
    ) -> anyhow::Result<()> {
        let group: Group = serde_json::from_value(value)?;

        assert_eq!(group.id, GroupId::new("1"));
        assert_eq!(group.name, None);
        assert_eq!(group.group_type, None);
        assert_eq!(group.created_at, None);
        assert_eq!(group.invitability_level, None);
        assert_eq!(group.permissions, None);

        Ok(())
    }

    /// グループの種類を、BOXクライアントが知らない種類も含めてデシリアライズすることを確認
    #[rstest]
    #[case::managed("managed_group", GroupType::ManagedGroup)]
    #[case::all_users("all_users_group", GroupType::AllUsersGroup)]
    #[case::unknown("new_group_type", GroupType::Unknown)]
    fn test_deserialize_group_type(
        #[case] value: &str,
        #[case] expected: GroupType,
    ) -> anyhow::Result<()> {
        let group_type: GroupType = serde_json::from_value(json!(value))?;

        assert_eq!(group_type, expected);

        Ok(())
    }

    /// 操作できるユーザーの範囲を、BOXクライアントが知らない範囲も含めてデシリアライズすることを確認
    #[rstest]
    #[case::admins_only("admins_only", GroupAccessLevel::AdminsOnly)]
    #[case::admins_and_members("admins_and_members", GroupAccessLevel::AdminsAndMembers)]
    #[case::all_managed_users("all_managed_users", GroupAccessLevel::AllManagedUsers)]
    #[case::unknown("everyone", GroupAccessLevel::Unknown)]
    fn test_deserialize_group_access_level(
        #[case] value: &str,
        #[case] expected: GroupAccessLevel,
    ) -> anyhow::Result<()> {
        let level: GroupAccessLevel = serde_json::from_value(json!(value))?;

        assert_eq!(level, expected);

        Ok(())
    }
}

#[cfg(test)]
mod test_group_information {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// `GET /groups/{group_id}`を呼び出し、グループ情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_group_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/groups/57645"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "group",
                "id": "57645",
                "name": "Support",
                "group_type": "managed_group",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let group = client.group_information(&GroupId::new("57645")).await?;

        assert_eq!(group.id, GroupId::new("57645"));
        assert_eq!(group.name.as_deref(), Some("Support"));
        assert_eq!(group.group_type, Some(GroupType::ManagedGroup));

        Ok(())
    }
}

#[cfg(test)]
mod test_list_enterprise_groups {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{
        create_client, mount_token, request_count, request_queries, short_retry,
    };

    /// グループのAPIのパス
    const GROUPS_PATH: &str = "/2.0/groups";

    /// オフセットとオプションをクエリパラメーターで送信し、グループの1ページをデシリアライズすることを確認
    #[rstest]
    #[case::default(0, ListEnterpriseGroupsOptions::default(), "offset=0")]
    #[case::all_options(
        2,
        ListEnterpriseGroupsOptions::default()
            .with_filter_term("Engineering")
            .with_limit(100),
        "offset=2&limit=100&filter_term=Engineering"
    )]
    #[case::offset_near_max(
        MAX_LISTED_GROUPS - 2,
        ListEnterpriseGroupsOptions::default(),
        "offset=9998"
    )]
    #[tokio::test]
    async fn test_list_enterprise_groups(
        #[case] offset: u64,
        #[case] options: ListEnterpriseGroupsOptions<'_>,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(GROUPS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [
                    {"type": "group", "id": "1", "name": "Engineering"},
                    {"type": "group", "id": "2", "name": "Engineering Support"},
                ],
                "total_count": 5,
                "limit": 100,
                "offset": 2,
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client.list_enterprise_groups(offset, &options).await?;

        let ids: Vec<_> = page.entries.iter().map(|g| g.id.as_ref()).collect();
        assert_eq!(ids, vec!["1", "2"]);
        assert_eq!(page.total_count, 5);
        assert_eq!(page.limit, Some(100));
        assert_eq!(page.offset, Some(2));
        assert_eq!(
            request_queries(&server, GROUPS_PATH).await,
            vec![expected_query]
        );

        Ok(())
    }

    /// BOXが`limit`と`offset`を含めずに返したレスポンスでも、ページをデシリアライズすることを確認
    #[tokio::test]
    async fn test_list_enterprise_groups_without_limit_and_offset() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(GROUPS_PATH))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"entries": [], "total_count": 0})),
            )
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client
            .list_enterprise_groups(0, &ListEnterpriseGroupsOptions::default())
            .await?;

        assert_eq!(page.entries, Vec::new());
        assert_eq!(page.total_count, 0);
        assert_eq!(page.limit, None);
        assert_eq!(page.offset, None);

        Ok(())
    }

    /// 先頭から上限の件数までしか返さないように、上限を超えるグループをページから取り除くことを確認
    #[rstest]
    #[case::within_max(9998, 2)]
    #[case::truncated_at_max(9999, 1)]
    #[tokio::test]
    async fn test_list_enterprise_groups_truncates_at_max(
        #[case] offset: u64,
        #[case] expected_entries: usize,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(GROUPS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "group", "id": "1"}, {"type": "group", "id": "2"}],
                "total_count": 20_000,
                "limit": 2,
                "offset": offset,
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client
            .list_enterprise_groups(
                offset,
                &ListEnterpriseGroupsOptions::default().with_limit(2),
            )
            .await?;

        assert_eq!(page.entries.len(), expected_entries);

        Ok(())
    }

    /// オフセットまたは上限が範囲外の場合は、リクエストを送信せずに`InvalidArgument`を返すことを確認
    #[rstest]
    #[case::offset_at_max(MAX_LISTED_GROUPS, ListEnterpriseGroupsOptions::default())]
    #[case::offset_above_max(MAX_LISTED_GROUPS + 1, ListEnterpriseGroupsOptions::default())]
    #[case::zero_limit(0, ListEnterpriseGroupsOptions::default().with_limit(0))]
    #[case::limit_above_max(
        0,
        ListEnterpriseGroupsOptions::default().with_limit(MAX_ITEMS_LIMIT + 1)
    )]
    #[tokio::test]
    async fn test_list_enterprise_groups_invalid_arguments(
        #[case] offset: u64,
        #[case] options: ListEnterpriseGroupsOptions<'_>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = client.list_enterprise_groups(offset, &options).await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }
}

#[cfg(test)]
mod test_list_all_enterprise_groups {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// グループのAPIのパス
    const GROUPS_PATH: &str = "/2.0/groups";

    /// オフセットを進めながらすべてのページを取得し、上限の既定値とオプションを各ページで送信することを確認
    #[tokio::test]
    async fn test_list_all_enterprise_groups() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(GROUPS_PATH))
            .and(query_param("offset", "0"))
            .and(query_param("limit", "1000"))
            .and(query_param("filter_term", "Eng"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "group", "id": "1"}, {"type": "group", "id": "2"}],
                "total_count": 3,
                "limit": 1000,
                "offset": 0,
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(GROUPS_PATH))
            .and(query_param("offset", "2"))
            .and(query_param("limit", "1000"))
            .and(query_param("filter_term", "Eng"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "group", "id": "3"}],
                "total_count": 3,
                "limit": 1000,
                "offset": 2,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let groups = client
            .list_all_enterprise_groups(
                &ListEnterpriseGroupsOptions::default().with_filter_term("Eng"),
            )
            .await?;

        let ids: Vec<_> = groups.iter().map(|g| g.id.as_ref()).collect();
        assert_eq!(ids, vec!["1", "2", "3"]);

        Ok(())
    }

    /// グループが上限より多い場合は、エラーにせず、先頭から上限の件数までを返すことを確認
    ///
    /// 上限が1ページの件数で割り切れない場合も、最後のページの余分なグループを取り除き、上限ちょうどの件数を返す。
    #[rstest]
    #[case::limit_divides_max(1000, 10)]
    #[case::limit_does_not_divide_max(300, 34)]
    #[tokio::test]
    async fn test_list_all_enterprise_groups_stops_at_max(
        #[case] limit: u32,
        #[case] expected_requests: u64,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(GROUPS_PATH))
            .respond_with(move |request: &wiremock::Request| {
                let offset: u64 = request
                    .url
                    .query_pairs()
                    .find(|(key, _)| key == "offset")
                    .and_then(|(_, value)| value.parse().ok())
                    .unwrap_or_default();
                let entries: Vec<_> = (offset..offset + u64::from(limit))
                    .map(|id| json!({"type": "group", "id": id.to_string()}))
                    .collect();
                ResponseTemplate::new(200).set_body_json(json!({
                    "entries": entries,
                    "total_count": 20_000,
                    "limit": limit,
                    "offset": offset,
                }))
            })
            .expect(expected_requests)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let groups = client
            .list_all_enterprise_groups(&ListEnterpriseGroupsOptions::default().with_limit(limit))
            .await?;

        assert_eq!(groups.len() as u64, MAX_LISTED_GROUPS);
        assert_eq!(groups.first().map(|g| g.id.as_ref()), Some("0"));
        assert_eq!(groups.last().map(|g| g.id.as_ref()), Some("9999"));

        Ok(())
    }

    /// BOXが空のページを返した場合は、`total_count`に達していなくても終了することを確認
    #[tokio::test]
    async fn test_list_all_enterprise_groups_stops_at_empty_page() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(GROUPS_PATH))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "group", "id": "1"}],
                "total_count": 5,
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(GROUPS_PATH))
            .and(query_param("offset", "1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"entries": [], "total_count": 5})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let groups = client
            .list_all_enterprise_groups(&ListEnterpriseGroupsOptions::default())
            .await?;

        assert_eq!(groups.len(), 1);

        Ok(())
    }

    /// 上限を指定した場合は、その値を各ページで送信することを確認
    #[tokio::test]
    async fn test_list_all_enterprise_groups_with_limit() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(GROUPS_PATH))
            .and(query_param("limit", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "group", "id": "1"}],
                "total_count": 1,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let groups = client
            .list_all_enterprise_groups(&ListEnterpriseGroupsOptions::default().with_limit(10))
            .await?;

        assert_eq!(groups.len(), 1);

        Ok(())
    }
}

#[cfg(test)]
mod test_create_group {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_not_retried, create_client, is_invalid_argument, is_invalid_request_body,
        mount_token, request_count, short_retry,
    };

    /// グループのAPIのパス
    const GROUPS_PATH: &str = "/2.0/groups";

    /// 作成したグループのレスポンスを返す。
    fn created_response() -> ResponseTemplate {
        ResponseTemplate::new(201).set_body_json(json!({
            "type": "group",
            "id": "999",
            "name": "Support",
        }))
    }

    /// 名前とオプションをリクエストボディに変換し、作成したグループを返すことを確認
    #[rstest]
    #[case::minimal(CreateGroupOptions::default(), json!({"name": "Support"}))]
    #[case::all_options(
        CreateGroupOptions::default()
            .with_provenance("Active Directory")
            .with_external_sync_identifier("AD:123456")
            .with_description("サポート担当")
            .with_invitability_level(GroupAccessLevel::AdminsAndMembers)
            .with_member_viewability_level(GroupAccessLevel::AllManagedUsers),
        json!({
            "name": "Support",
            "provenance": "Active Directory",
            "external_sync_identifier": "AD:123456",
            "description": "サポート担当",
            "invitability_level": "admins_and_members",
            "member_viewability_level": "all_managed_users",
        })
    )]
    #[tokio::test]
    async fn test_create_group(
        #[case] options: CreateGroupOptions<'_>,
        #[case] expected_body: serde_json::Value,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(GROUPS_PATH))
            .and(body_json(expected_body))
            .respond_with(created_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let group = client.create_group("Support", &options).await?;

        assert_eq!(group.id, GroupId::new("999"));

        Ok(())
    }

    /// 名前またはオプションをBOXが拒否する場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::empty_name("", CreateGroupOptions::default(), is_invalid_request_body)]
    #[case::too_long_provenance(
        "Support",
        CreateGroupOptions::default()
            .with_provenance("a".repeat(MAX_GROUP_PROVENANCE_CHARS + 1).leak()),
        is_invalid_request_body
    )]
    #[case::too_long_description(
        "Support",
        CreateGroupOptions::default()
            .with_description("a".repeat(MAX_GROUP_DESCRIPTION_CHARS + 1).leak()),
        is_invalid_request_body
    )]
    #[case::unknown_invitability_level(
        "Support",
        CreateGroupOptions::default().with_invitability_level(GroupAccessLevel::Unknown),
        is_invalid_argument
    )]
    #[case::unknown_member_viewability_level(
        "Support",
        CreateGroupOptions::default().with_member_viewability_level(GroupAccessLevel::Unknown),
        is_invalid_argument
    )]
    #[tokio::test]
    async fn test_create_group_invalid_arguments(
        #[case] name: &str,
        #[case] options: CreateGroupOptions<'_>,
        #[case] is_expected_error: fn(&BoxClientError) -> bool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = client.create_group(name, &options).await;

        let Err(error) = result else {
            panic!("result: {result:?}");
        };
        assert!(is_expected_error(&error), "error: {error:?}");
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// 上限の文字数ちょうどの出所と説明を受け付けることを確認
    #[tokio::test]
    async fn test_create_group_accepts_max_lengths() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(GROUPS_PATH))
            .respond_with(created_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;
        let (provenance, description) = (
            "あ".repeat(MAX_GROUP_PROVENANCE_CHARS),
            "あ".repeat(MAX_GROUP_DESCRIPTION_CHARS),
        );

        client
            .create_group(
                "Support",
                &CreateGroupOptions::default()
                    .with_provenance(&provenance)
                    .with_description(&description),
            )
            .await?;

        Ok(())
    }

    /// グループを重複して作成しないように、5xxを受け取っても再試行しないことを確認
    #[rstest]
    #[case::conflict(409)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_create_group_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        assert_not_retried("POST", GROUPS_PATH, status, async |client: &BoxClient| {
            client
                .create_group("Support", &CreateGroupOptions::default())
                .await
        })
        .await
    }
}

#[cfg(test)]
mod test_update_group {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_retried_after, create_client, is_invalid_argument, is_invalid_request_body,
        mount_token, request_count, short_retry,
    };

    /// グループのAPIのパス
    const GROUP_PATH: &str = "/2.0/groups/57645";

    /// 更新したグループのレスポンスを返す。
    fn updated_response() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "type": "group",
            "id": "57645",
            "name": "Customer Support",
        }))
    }

    /// 変更する属性をリクエストボディに変換し、更新したグループを返すことを確認
    #[rstest]
    #[case::empty(GroupUpdate::default(), json!({}))]
    #[case::all_attributes(
        GroupUpdate::default()
            .with_name("Customer Support")
            .with_provenance("Okta")
            .with_external_sync_identifier("okta:1")
            .with_description("顧客サポート担当")
            .with_invitability_level(GroupAccessLevel::AllManagedUsers)
            .with_member_viewability_level(GroupAccessLevel::AdminsOnly),
        json!({
            "name": "Customer Support",
            "provenance": "Okta",
            "external_sync_identifier": "okta:1",
            "description": "顧客サポート担当",
            "invitability_level": "all_managed_users",
            "member_viewability_level": "admins_only",
        })
    )]
    #[tokio::test]
    async fn test_update_group(
        #[case] update: GroupUpdate<'_>,
        #[case] expected_body: serde_json::Value,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(GROUP_PATH))
            .and(body_json(expected_body))
            .respond_with(updated_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let group = client.update_group(&GroupId::new("57645"), &update).await?;

        assert_eq!(group.id, GroupId::new("57645"));
        assert_eq!(group.name.as_deref(), Some("Customer Support"));

        Ok(())
    }

    /// 変更する属性をBOXが拒否する場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::empty_name(GroupUpdate::default().with_name(""), is_invalid_request_body)]
    #[case::too_long_provenance(
        GroupUpdate::default()
            .with_provenance("a".repeat(MAX_GROUP_PROVENANCE_CHARS + 1).leak()),
        is_invalid_request_body
    )]
    #[case::too_long_description(
        GroupUpdate::default()
            .with_description("a".repeat(MAX_GROUP_DESCRIPTION_CHARS + 1).leak()),
        is_invalid_request_body
    )]
    #[case::unknown_invitability_level(
        GroupUpdate::default().with_invitability_level(GroupAccessLevel::Unknown),
        is_invalid_argument
    )]
    #[case::unknown_member_viewability_level(
        GroupUpdate::default().with_member_viewability_level(GroupAccessLevel::Unknown),
        is_invalid_argument
    )]
    #[tokio::test]
    async fn test_update_group_invalid_arguments(
        #[case] update: GroupUpdate<'_>,
        #[case] is_expected_error: fn(&BoxClientError) -> bool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = client.update_group(&GroupId::new("57645"), &update).await;

        let Err(error) = result else {
            panic!("result: {result:?}");
        };
        assert!(is_expected_error(&error), "error: {error:?}");
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// 冪等なリクエストとして、5xxの後に再試行することを確認
    #[tokio::test]
    async fn test_update_group_retries_after_server_error() -> anyhow::Result<()> {
        assert_retried_after(
            "PUT",
            GROUP_PATH,
            ResponseTemplate::new(500),
            updated_response(),
            async |client: &BoxClient| {
                client
                    .update_group(
                        &GroupId::new("57645"),
                        &GroupUpdate::default().with_description("顧客サポート担当"),
                    )
                    .await
            },
        )
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod test_delete_group {
    use rstest::rstest;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_not_retried, assert_retried_after, count_requests, create_client, mount_token,
        operation_blocked_response, short_retry,
    };

    /// グループのAPIのパス
    const GROUP_PATH: &str = "/2.0/groups/57645";

    /// `DELETE /groups/{group_id}`を呼び出し、204を受け取ると成功することを確認
    #[tokio::test]
    async fn test_delete_group() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(GROUP_PATH))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        client.delete_group(&GroupId::new("57645")).await?;

        assert_eq!(count_requests(&server, "DELETE", GROUP_PATH).await, 1);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 5xxの後に再送すると、削除済みのグループに対してエラーを受け取るおそれがあるため、
    /// 再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::forbidden(403)]
    #[case::not_found(404)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_delete_group_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        assert_not_retried("DELETE", GROUP_PATH, status, async |client: &BoxClient| {
            client.delete_group(&GroupId::new("57645")).await
        })
        .await
    }

    /// 429または他の操作によるロックで409を受け取った場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[rstest]
    #[case::too_many_requests(ResponseTemplate::new(429))]
    #[case::operation_blocked_temporary(operation_blocked_response())]
    #[tokio::test]
    async fn test_delete_group_retries_request_not_processed(
        #[case] first_response: ResponseTemplate,
    ) -> anyhow::Result<()> {
        assert_retried_after(
            "DELETE",
            GROUP_PATH,
            first_response,
            ResponseTemplate::new(204),
            async |client: &BoxClient| client.delete_group(&GroupId::new("57645")).await,
        )
        .await?;

        Ok(())
    }
}
