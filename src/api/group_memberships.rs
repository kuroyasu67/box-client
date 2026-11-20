//! グループメンバーシップに関するAPI
//!
//! グループメンバーシップは、ユーザーがグループに所属していることを表す。
//! グループメンバーシップの作成は、管理者の権限を持つユーザーまたはアプリケーションだけが実行できる。

use serde::{Deserialize, Serialize, Serializer, ser::SerializeMap};
use time::OffsetDateTime;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, GroupId, GroupMembershipId, GroupMini, Nullable,
    UserId, UserMini,
    api::{
        pagination::{MAX_ITEMS_LIMIT, OffsetPage, collect_offset_pages_up_to, validate_limit},
        updates::NullableBody,
    },
    request::ApiRequest,
    retry::Idempotency,
};

/// グループメンバーシップをリストするときに取得できるメンバーシップ数の上限を表す整数リテラルに展開する。
///
/// `concat!`は定数を受け付けないため、[`MAX_LISTED_GROUP_MEMBERSHIPS`]とエラーのメッセージで、このマクロを共有する。
/// これにより、上限を変更したときに、メッセージの値も追随させる。
macro_rules! max_listed_group_memberships {
    () => {
        10_000
    };
}

/// グループメンバーシップをリストするときに取得できるメンバーシップ数の上限
///
/// BOXは、グループのメンバーのリストおよびユーザーが所属するグループのリストをオフセット方式でだけページ送りし、
/// オフセットが10,000を超えるリクエストを400で拒否する。そのため、BOXクライアントは、
/// メンバーシップのうち、先頭からこの件数までしか返さない。
pub const MAX_LISTED_GROUP_MEMBERSHIPS: u64 = max_listed_group_memberships!();

/// グループメンバーシップ
///
/// BOXのAPI仕様（OpenAPI）は、すべての属性を省略可能としている。
/// ただし、メンバーシップを操作するにはIDが必要であるため、IDだけは必須とし、
/// BOXが属性を省略したりnullを返したりしても取得を失敗させないように、ID以外の属性を`Option`で受け取る。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct GroupMembership {
    /// グループメンバーシップID
    pub id: GroupMembershipId,

    /// グループに所属するユーザー
    pub user: Option<UserMini>,

    /// ユーザーが所属するグループ
    pub group: Option<GroupMini>,

    /// グループでのユーザーの役割
    pub role: Option<GroupMembershipRole>,

    /// 作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,

    /// 最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,
}

/// グループでのユーザーの役割
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GroupMembershipRole {
    /// メンバー
    Member,

    /// グループの管理者
    Admin,

    /// BOXクライアントが知らない役割
    ///
    /// BOXが役割を追加しても、グループメンバーシップの取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

impl GroupMembershipRole {
    /// リクエストボディに指定する文字列を返す。
    ///
    /// [`GroupMembershipRole::Unknown`]は、BOXに送信する値を持たないため、`None`を返す。
    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Member => Some("member"),
            Self::Admin => Some("admin"),
            Self::Unknown => None,
        }
    }
}

/// グループでのユーザーの役割をリクエストボディの値に変換する。
///
/// # Errors
///
/// `role`が[`GroupMembershipRole::Unknown`]の場合は、[`BoxClientError::InvalidArgument`]を返す。
fn role_str(role: Option<GroupMembershipRole>) -> BoxClientResult<Option<&'static str>> {
    role.map(|role| {
        role.as_str().ok_or(BoxClientError::InvalidArgument(
            "role of a group membership must not be unknown",
        ))
    })
    .transpose()
}

/// グループの管理者に与える権限の設定
///
/// 権限の名前（例えば`can_run_reports`）と、その権限を与えるかどうかの組を並べる。
/// BOXのドキュメントによると、指定しない権限は有効になる。また、役割が[`GroupMembershipRole::Member`]の
/// ユーザーには効果がない。
pub type ConfigurablePermissions<'a> = &'a [(&'a str, bool)];

/// グループの管理者に与える権限の設定のリクエストボディ
///
/// BOXは、権限の名前をキー、権限を与えるかどうかを値とするオブジェクトを受け付けるため、組の並びをオブジェクトにシリアライズする。
#[derive(Debug)]
struct ConfigurablePermissionsBody<'a>(ConfigurablePermissions<'a>);

impl Serialize for ConfigurablePermissionsBody<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, enabled) in self.0 {
            map.serialize_entry(name, enabled)?;
        }
        map.end()
    }
}

/// グループの管理者に与える権限の設定を検証し、リクエストボディの値に変換する。
///
/// # Errors
///
/// 権限の名前が空文字列または空白だけの場合、または同じ権限の名前が重複している場合は、
/// [`BoxClientError::InvalidArgument`]を返す。
fn permissions_body(
    permissions: Option<Nullable<ConfigurablePermissions<'_>>>,
) -> BoxClientResult<Option<NullableBody<ConfigurablePermissionsBody<'_>>>> {
    if let Some(Nullable::Value(permissions)) = permissions {
        for (index, (name, _)) in permissions.iter().enumerate() {
            if name.trim().is_empty() {
                return Err(BoxClientError::InvalidArgument(
                    "name of a configurable permission must not be empty",
                ));
            }
            if permissions[..index].iter().any(|(other, _)| other == name) {
                return Err(BoxClientError::InvalidArgument(
                    "configurable permissions must not contain duplicate names",
                ));
            }
        }
    }
    Ok(permissions.map(|permissions| permissions.to_body(|&p| ConfigurablePermissionsBody(p))))
}

/// グループメンバーシップの作成のオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// `None`の属性は送信せず、BOXの既定値を使用する。
///
/// # Examples
///
/// ```
/// use box_client::{CreateGroupMembershipOptions, GroupMembershipRole};
///
/// let options = CreateGroupMembershipOptions::default()
///     .with_role(GroupMembershipRole::Admin)
///     .with_configurable_permissions(&[("can_run_reports", false)]);
/// assert_eq!(options.role, Some(GroupMembershipRole::Admin));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CreateGroupMembershipOptions<'a> {
    /// グループでのユーザーの役割
    ///
    /// [`GroupMembershipRole::Unknown`]は指定できない。
    pub role: Option<GroupMembershipRole>,

    /// グループの管理者に与える権限の設定
    ///
    /// [`Nullable::Value`]を指定すると権限を設定し、[`Nullable::Null`]を指定するとすべての権限の設定を無効にする。
    pub configurable_permissions: Option<Nullable<ConfigurablePermissions<'a>>>,
}

impl<'a> CreateGroupMembershipOptions<'a> {
    /// グループでのユーザーの役割を設定する。
    #[must_use]
    pub fn with_role(mut self, role: GroupMembershipRole) -> Self {
        self.role = Some(role);
        self
    }

    /// グループの管理者に与える権限を設定する。
    #[must_use]
    pub fn with_configurable_permissions(
        mut self,
        permissions: ConfigurablePermissions<'a>,
    ) -> Self {
        self.configurable_permissions = Some(Nullable::Value(permissions));
        self
    }

    /// グループの管理者に与える権限の設定をすべて無効にする。
    #[must_use]
    pub fn with_configurable_permissions_disabled(mut self) -> Self {
        self.configurable_permissions = Some(Nullable::Null);
        self
    }
}

/// グループメンバーシップの更新で変更する属性
///
/// `None`の属性は送信せず、BOXは変更しない。
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// # Examples
///
/// ```
/// use box_client::{GroupMembershipRole, GroupMembershipUpdate};
///
/// let update = GroupMembershipUpdate::default().with_role(GroupMembershipRole::Member);
/// assert_eq!(update.role, Some(GroupMembershipRole::Member));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct GroupMembershipUpdate<'a> {
    /// グループでのユーザーの役割
    ///
    /// [`GroupMembershipRole::Unknown`]は指定できない。
    pub role: Option<GroupMembershipRole>,

    /// グループの管理者に与える権限の設定
    ///
    /// [`Nullable::Value`]を指定すると権限を設定し、[`Nullable::Null`]を指定するとすべての権限の設定を無効にする。
    pub configurable_permissions: Option<Nullable<ConfigurablePermissions<'a>>>,
}

impl<'a> GroupMembershipUpdate<'a> {
    /// グループでのユーザーの役割を設定する。
    #[must_use]
    pub fn with_role(mut self, role: GroupMembershipRole) -> Self {
        self.role = Some(role);
        self
    }

    /// グループの管理者に与える権限を設定する。
    #[must_use]
    pub fn with_configurable_permissions(
        mut self,
        permissions: ConfigurablePermissions<'a>,
    ) -> Self {
        self.configurable_permissions = Some(Nullable::Value(permissions));
        self
    }

    /// グループの管理者に与える権限の設定をすべて無効にする。
    #[must_use]
    pub fn with_configurable_permissions_disabled(mut self) -> Self {
        self.configurable_permissions = Some(Nullable::Null);
        self
    }
}

/// IDだけを持つユーザーまたはグループのリクエストボディ
#[derive(Debug, Serialize)]
struct IdBody<'a> {
    /// ユーザーまたはグループのID
    id: &'a str,
}

/// グループメンバーシップの作成リクエストのボディ
#[derive(Debug, Serialize)]
struct CreateGroupMembershipBody<'a> {
    /// グループに追加するユーザー
    user: IdBody<'a>,

    /// ユーザーを追加するグループ
    group: IdBody<'a>,

    /// グループでのユーザーの役割
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,

    /// グループの管理者に与える権限の設定
    #[serde(skip_serializing_if = "Option::is_none")]
    configurable_permissions: Option<NullableBody<ConfigurablePermissionsBody<'a>>>,
}

impl<'a> CreateGroupMembershipBody<'a> {
    /// 引数とオプションからボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// `options.role`が[`GroupMembershipRole::Unknown`]の場合、
    /// または権限の名前が空文字列、空白だけもしくは重複している場合は、[`BoxClientError::InvalidArgument`]を返す。
    fn new(
        user_id: &'a UserId,
        group_id: &'a GroupId,
        options: &CreateGroupMembershipOptions<'a>,
    ) -> BoxClientResult<Self> {
        Ok(Self {
            user: IdBody {
                id: user_id.as_ref(),
            },
            group: IdBody {
                id: group_id.as_ref(),
            },
            role: role_str(options.role)?,
            configurable_permissions: permissions_body(options.configurable_permissions)?,
        })
    }
}

/// グループメンバーシップの更新リクエストのボディ
#[derive(Debug, Serialize)]
struct UpdateGroupMembershipBody<'a> {
    /// グループでのユーザーの役割
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,

    /// グループの管理者に与える権限の設定
    #[serde(skip_serializing_if = "Option::is_none")]
    configurable_permissions: Option<NullableBody<ConfigurablePermissionsBody<'a>>>,
}

impl<'a> UpdateGroupMembershipBody<'a> {
    /// 変更する属性からボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// [`CreateGroupMembershipBody::new`]と同じ場合にエラーを返す。
    fn new(update: &GroupMembershipUpdate<'a>) -> BoxClientResult<Self> {
        Ok(Self {
            role: role_str(update.role)?,
            configurable_permissions: permissions_body(update.configurable_permissions)?,
        })
    }
}

/// グループメンバーシップをリストするときのオプション
///
/// グループのメンバーのリストとユーザーが所属するグループのリストで共有する。
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// BOXクライアントは、メンバーシップのうち、先頭から[`MAX_LISTED_GROUP_MEMBERSHIPS`]件までしか返さない。
/// 理由は[`BoxClient::list_group_memberships`]を参照する。
///
/// # Examples
///
/// ```
/// use box_client::ListGroupMembershipsOptions;
///
/// let options = ListGroupMembershipsOptions::default().with_limit(100);
/// assert_eq!(options.limit, Some(100));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListGroupMembershipsOptions {
    /// 1ページで取得するメンバーシップ数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,
}

impl ListGroupMembershipsOptions {
    /// 1ページで取得するメンバーシップ数の上限を設定する。
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。範囲外の値は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// グループメンバーシップの1ページ
///
/// BOXは、グループのメンバーのリストおよびユーザーが所属するグループのリストをオフセット方式でページ送りする。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct GroupMembershipsPage {
    /// このページのメンバーシップ
    pub entries: Vec<GroupMembership>,

    /// 最後のメンバーシップのオフセットに1を加えた値
    pub total_count: u64,

    /// このページの取得に使用したメンバーシップ数の上限
    ///
    /// BOXがレスポンスに含めない場合は`None`になる。
    pub limit: Option<u32>,

    /// このページの最初のメンバーシップのオフセット（0始まり）
    ///
    /// BOXがレスポンスに含めない場合は`None`になる。
    pub offset: Option<u64>,
}

impl OffsetPage for GroupMembershipsPage {
    type Entry = GroupMembership;

    fn into_parts(self) -> (Vec<GroupMembership>, u64) {
        (self.entries, self.total_count)
    }
}

/// オフセットが、取得できるメンバーシップ数の上限より小さいことを検証する。
///
/// BOXクライアントは、先頭から[`MAX_LISTED_GROUP_MEMBERSHIPS`]件までしか返さないため、それ以降のオフセットを受け付けない。
///
/// # Errors
///
/// `offset`が[`MAX_LISTED_GROUP_MEMBERSHIPS`]以上の場合は、[`BoxClientError::InvalidArgument`]を返す。
fn validate_memberships_offset(offset: u64) -> BoxClientResult<u64> {
    if MAX_LISTED_GROUP_MEMBERSHIPS <= offset {
        return Err(BoxClientError::InvalidArgument(concat!(
            "offset must be less than ",
            max_listed_group_memberships!()
        )));
    }
    Ok(offset)
}

impl BoxClient {
    /// グループメンバーシップの情報を取得する。
    ///
    /// BOXのドキュメントによると、グループの管理者、または管理者の権限を持つユーザーだけが実行できる。
    ///
    /// # Arguments
    ///
    /// - `group_membership_id`: 情報を取得するグループメンバーシップのID
    ///
    /// # Errors
    ///
    /// グループメンバーシップが存在しない場合、またはグループメンバーシップを参照する権限がない場合は、
    /// [`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn group_membership_information(
        &self,
        group_membership_id: &GroupMembershipId,
    ) -> BoxClientResult<GroupMembership> {
        self.send_json(ApiRequest::get([
            "group_memberships",
            group_membership_id.as_ref(),
        ]))
        .await
    }

    /// グループのメンバーを1ページ分リストする。
    ///
    /// BOXのドキュメントによると、グループのメンバー、または管理者の権限を持つユーザーだけが実行できる。
    /// メンバーがいない場合は、空のページを返す。
    ///
    /// BOXは、このAPIをオフセット方式でだけページ送りし、マーカー方式に対応しない。
    /// また、BOXはオフセットが10,000を超えるリクエストを拒否し、それより後ろのメンバーシップを取得できない。
    /// そのため、BOXクライアントは、メンバーシップのうち、先頭から[`MAX_LISTED_GROUP_MEMBERSHIPS`]件までしか返さない。
    /// 上限を超えるメンバーシップは、ページから取り除く。
    ///
    /// 1回の呼び出しで返すメンバーシップは最大で`limit`件である。次のページを取得する場合は、`offset`に、
    /// 前のページの`offset`と`entries`の数の和を指定して再度呼び出す。その和が[`MAX_LISTED_GROUP_MEMBERSHIPS`]に
    /// 達した場合は、後続のメンバーシップが残っていても、それ以上取得できない。
    /// すべてのメンバーシップが必要な場合は、[`BoxClient::list_all_group_memberships`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `group_id`: メンバーをリストするグループのID
    /// - `offset`: 取得する最初のメンバーシップのオフセット（0始まり）。[`MAX_LISTED_GROUP_MEMBERSHIPS`]より小さい値を指定する。
    /// - `options`: メンバーシップ数の上限のオプション
    ///
    /// # Errors
    ///
    /// `offset`が[`MAX_LISTED_GROUP_MEMBERSHIPS`]以上の場合、または`options.limit`が範囲外の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// グループが存在しない場合、またはグループのメンバーを参照する権限がない場合は、
    /// [`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_group_memberships(
        &self,
        group_id: &GroupId,
        offset: u64,
        options: &ListGroupMembershipsOptions,
    ) -> BoxClientResult<GroupMembershipsPage> {
        self.list_memberships_page(
            ["groups", group_id.as_ref(), "memberships"],
            offset,
            options,
        )
        .await
    }

    /// グループのすべてのメンバーを、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。
    /// 取得したメンバーシップの数が`total_count`もしくは[`MAX_LISTED_GROUP_MEMBERSHIPS`]に達した場合、
    /// またはBOXが空のページを返した場合に終了する。
    ///
    /// [`BoxClient::list_group_memberships`]と同じ理由で、メンバーシップのうち、先頭から
    /// [`MAX_LISTED_GROUP_MEMBERSHIPS`]件までしか返さない。メンバーシップがそれより多くても、エラーにはせず、
    /// 先頭から[`MAX_LISTED_GROUP_MEMBERSHIPS`]件を返す。
    ///
    /// # Arguments
    ///
    /// - `group_id`: メンバーをリストするグループのID
    /// - `options`: メンバーシップ数の上限のオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みのメンバーシップを破棄してエラーを返す。
    pub async fn list_all_group_memberships(
        &self,
        group_id: &GroupId,
        options: &ListGroupMembershipsOptions,
    ) -> BoxClientResult<Vec<GroupMembership>> {
        let options = ListGroupMembershipsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
        };
        let (memberships, pages) =
            collect_offset_pages_up_to(MAX_LISTED_GROUP_MEMBERSHIPS, async |offset| {
                self.list_group_memberships(group_id, offset, &options)
                    .await
            })
            .await?;

        tracing::debug!(
            group_id = group_id.as_ref(),
            pages,
            memberships = memberships.len(),
            "listed all memberships of group"
        );

        Ok(memberships)
    }

    /// ユーザーが所属するグループを1ページ分リストする。
    ///
    /// BOXのドキュメントによると、グループのメンバー、または管理者の権限を持つユーザーだけが実行できる。
    /// ユーザーがどのグループにも所属していない場合は、空のページを返す。
    ///
    /// [`BoxClient::list_group_memberships`]と同じ理由で、メンバーシップのうち、先頭から
    /// [`MAX_LISTED_GROUP_MEMBERSHIPS`]件までしか返さない。ページ送りの方法も同じである。
    /// すべてのメンバーシップが必要な場合は、[`BoxClient::list_all_user_memberships`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `user_id`: 所属するグループをリストするユーザーのID
    /// - `offset`: 取得する最初のメンバーシップのオフセット（0始まり）。[`MAX_LISTED_GROUP_MEMBERSHIPS`]より小さい値を指定する。
    /// - `options`: メンバーシップ数の上限のオプション
    ///
    /// # Errors
    ///
    /// `offset`が[`MAX_LISTED_GROUP_MEMBERSHIPS`]以上の場合、または`options.limit`が範囲外の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// ユーザーが存在しない場合、またはユーザーが所属するグループを参照する権限がない場合は、
    /// [`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_user_memberships(
        &self,
        user_id: &UserId,
        offset: u64,
        options: &ListGroupMembershipsOptions,
    ) -> BoxClientResult<GroupMembershipsPage> {
        self.list_memberships_page(["users", user_id.as_ref(), "memberships"], offset, options)
            .await
    }

    /// ユーザーが所属するすべてのグループを、ページを送りながらリストする。
    ///
    /// [`BoxClient::list_all_group_memberships`]と同じく、`options.limit`が`None`の場合は1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得し、メンバーシップのうち、先頭から[`MAX_LISTED_GROUP_MEMBERSHIPS`]件までを返す。
    ///
    /// # Arguments
    ///
    /// - `user_id`: 所属するグループをリストするユーザーのID
    /// - `options`: メンバーシップ数の上限のオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みのメンバーシップを破棄してエラーを返す。
    pub async fn list_all_user_memberships(
        &self,
        user_id: &UserId,
        options: &ListGroupMembershipsOptions,
    ) -> BoxClientResult<Vec<GroupMembership>> {
        let options = ListGroupMembershipsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
        };
        let (memberships, pages) =
            collect_offset_pages_up_to(MAX_LISTED_GROUP_MEMBERSHIPS, async |offset| {
                self.list_user_memberships(user_id, offset, &options).await
            })
            .await?;

        tracing::debug!(
            user_id = user_id.as_ref(),
            pages,
            memberships = memberships.len(),
            "listed all memberships of user"
        );

        Ok(memberships)
    }

    /// ユーザーをグループに追加して、グループメンバーシップを作成する。
    ///
    /// BOXのドキュメントによると、管理者の権限を持つユーザーだけが実行できる。
    ///
    /// HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。
    /// 5xxまたはタイムアウトの後に再送すると、BOXがメンバーシップを作成済みの場合に、エラーを受け取るおそれがあるためである。
    ///
    /// # Arguments
    ///
    /// - `user_id`: グループに追加するユーザーのID
    /// - `group_id`: ユーザーを追加するグループのID
    /// - `options`: 役割およびグループの管理者に与える権限のオプション
    ///
    /// # Errors
    ///
    /// `options.role`が[`GroupMembershipRole::Unknown`]の場合、
    /// または権限の名前が空文字列、空白だけもしくは重複している場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 例えば、情報バリアによってユーザーをグループに追加することを禁止している場合は、BOXはエラーコードが
    /// `forbidden_by_policy`の403を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがメンバーシップを作成した可能性がある。
    /// したがって、[`BoxClient::list_group_memberships`]でグループのメンバーを確認する。
    pub async fn create_group_membership(
        &self,
        user_id: &UserId,
        group_id: &GroupId,
        options: &CreateGroupMembershipOptions<'_>,
    ) -> BoxClientResult<GroupMembership> {
        let body = CreateGroupMembershipBody::new(user_id, group_id, options)?;
        let request = ApiRequest::post(["group_memberships"]).json(&body)?;

        let membership: GroupMembership = self.send_json(request).await?;

        tracing::debug!(
            group_membership_id = membership.id.as_ref(),
            user_id = user_id.as_ref(),
            group_id = group_id.as_ref(),
            "created group membership"
        );

        Ok(membership)
    }

    /// グループメンバーシップを更新する。
    ///
    /// BOXのドキュメントによると、グループの管理者、または管理者の権限を持つユーザーだけが実行できる。
    ///
    /// 同じ値を何度設定しても結果が変わらないため、HTTPメソッドのとおり冪等なリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `group_membership_id`: 更新するグループメンバーシップのID
    /// - `update`: 変更する属性
    ///
    /// # Errors
    ///
    /// `update.role`が[`GroupMembershipRole::Unknown`]の場合、
    /// または権限の名前が空文字列、空白だけもしくは重複している場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn update_group_membership(
        &self,
        group_membership_id: &GroupMembershipId,
        update: &GroupMembershipUpdate<'_>,
    ) -> BoxClientResult<GroupMembership> {
        let body = UpdateGroupMembershipBody::new(update)?;
        let request =
            ApiRequest::put(["group_memberships", group_membership_id.as_ref()]).json(&body)?;

        let membership: GroupMembership = self.send_json(request).await?;

        tracing::debug!(
            group_membership_id = group_membership_id.as_ref(),
            "updated group membership"
        );

        Ok(membership)
    }

    /// グループメンバーシップを削除して、ユーザーをグループから外す。
    ///
    /// BOXのドキュメントによると、グループの管理者、または管理者の権限を持つユーザーだけが実行できる。
    ///
    /// BOXクライアントは、冪等なリクエストであれば、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがメンバーシップの削除を完了している可能性がある。
    /// BOXが削除した後にリクエストを再送すると、削除済みのメンバーシップに対する削除となるため、BOXはエラーを返しうる。
    /// その結果、削除に成功したにもかかわらず、削除に失敗したと誤って判断するおそれがある。
    /// したがって、HTTPメソッドがDELETEであっても冪等でないリクエストとして送信し、
    /// BOXがリクエストを処理していないことが明らかな429、エラーコードが`operation_blocked_temporary`の409
    /// および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `group_membership_id`: 削除するグループメンバーシップのID
    ///
    /// # Errors
    ///
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがメンバーシップを削除した可能性があるため、
    /// [`BoxClient::group_membership_information`]でメンバーシップが存在するか確認する。
    pub async fn delete_group_membership(
        &self,
        group_membership_id: &GroupMembershipId,
    ) -> BoxClientResult<()> {
        let request = ApiRequest::delete(["group_memberships", group_membership_id.as_ref()])
            .idempotency(Idempotency::NonIdempotent);

        self.send_empty(request).await?;

        tracing::debug!(
            group_membership_id = group_membership_id.as_ref(),
            "deleted group membership"
        );

        Ok(())
    }

    /// グループのメンバーのリストおよびユーザーが所属するグループのリストで、メンバーシップを1ページ分取得する。
    ///
    /// 先頭から[`MAX_LISTED_GROUP_MEMBERSHIPS`]件までしか返さないように、上限を超えるメンバーシップを取り除く。
    ///
    /// # Errors
    ///
    /// `offset`が[`MAX_LISTED_GROUP_MEMBERSHIPS`]以上の場合、または`options.limit`が範囲外の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    async fn list_memberships_page<'a>(
        &self,
        path: impl IntoIterator<Item = &'a str>,
        offset: u64,
        options: &ListGroupMembershipsOptions,
    ) -> BoxClientResult<GroupMembershipsPage> {
        let mut request =
            ApiRequest::get(path).query("offset", validate_memberships_offset(offset)?.to_string());
        if let Some(limit) = options.limit {
            request = request.query("limit", validate_limit(limit)?.to_string());
        }

        let mut page: GroupMembershipsPage = self.send_json(request).await?;
        page.entries
            .truncate(usize::try_from(MAX_LISTED_GROUP_MEMBERSHIPS - offset).unwrap_or(usize::MAX));
        Ok(page)
    }
}

#[cfg(test)]
mod test_group_membership {
    use rstest::rstest;
    use serde_json::json;
    use time::macros::datetime;

    use super::*;
    use crate::GroupType;

    /// すべての属性をデシリアライズすることを確認
    #[test]
    fn test_deserialize_group_membership() -> anyhow::Result<()> {
        let membership: GroupMembership = serde_json::from_value(json!({
            "id": "11446498",
            "type": "group_membership",
            "user": {"id": "1", "type": "user", "name": "Aaron Levie", "login": "ceo@example.com"},
            "group": {"id": "2", "type": "group", "name": "Support", "group_type": "managed_group"},
            "role": "member",
            "created_at": "2012-12-12T10:53:43-08:00",
            "modified_at": "2012-12-12T10:53:43-08:00",
        }))?;

        assert_eq!(membership.id, GroupMembershipId::new("11446498"));
        let user = membership
            .user
            .as_ref()
            .map(|u| (u.id.as_ref(), u.name.as_deref()));
        assert_eq!(user, Some(("1", Some("Aaron Levie"))));
        let group = membership
            .group
            .as_ref()
            .map(|g| (g.id.as_ref(), g.group_type));
        assert_eq!(group, Some(("2", Some(GroupType::ManagedGroup))));
        assert_eq!(membership.role, Some(GroupMembershipRole::Member));
        assert_eq!(
            membership.created_at,
            Some(datetime!(2012-12-12 10:53:43 -08:00))
        );
        assert_eq!(
            membership.modified_at,
            Some(datetime!(2012-12-12 10:53:43 -08:00))
        );

        Ok(())
    }

    /// ID以外の属性が省略またはnullの場合も、`None`としてデシリアライズすることを確認
    #[rstest]
    #[case::missing(json!({"id": "1"}))]
    #[case::null(json!({
        "id": "1",
        "type": "group_membership",
        "user": null,
        "group": null,
        "role": null,
        "created_at": null,
        "modified_at": null,
    }))]
    fn test_deserialize_group_membership_without_attributes(
        #[case] value: serde_json::Value,
    ) -> anyhow::Result<()> {
        let membership: GroupMembership = serde_json::from_value(value)?;

        assert_eq!(membership.id, GroupMembershipId::new("1"));
        assert_eq!(membership.user, None);
        assert_eq!(membership.group, None);
        assert_eq!(membership.role, None);
        assert_eq!(membership.created_at, None);

        Ok(())
    }

    /// 役割を、BOXクライアントが知らない役割も含めてデシリアライズすることを確認
    #[rstest]
    #[case::member("member", GroupMembershipRole::Member)]
    #[case::admin("admin", GroupMembershipRole::Admin)]
    #[case::unknown("owner", GroupMembershipRole::Unknown)]
    fn test_deserialize_group_membership_role(
        #[case] value: &str,
        #[case] expected: GroupMembershipRole,
    ) -> anyhow::Result<()> {
        let role: GroupMembershipRole = serde_json::from_value(json!(value))?;

        assert_eq!(role, expected);

        Ok(())
    }
}

#[cfg(test)]
mod test_group_membership_information {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// `GET /group_memberships/{group_membership_id}`を呼び出し、メンバーシップをデシリアライズすることを確認
    #[tokio::test]
    async fn test_group_membership_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/group_memberships/434534"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "group_membership",
                "id": "434534",
                "role": "admin",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let membership = client
            .group_membership_information(&GroupMembershipId::new("434534"))
            .await?;

        assert_eq!(membership.id, GroupMembershipId::new("434534"));
        assert_eq!(membership.role, Some(GroupMembershipRole::Admin));

        Ok(())
    }
}

#[cfg(test)]
mod test_list_memberships {
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

    /// グループのメンバーのAPIのパス
    const GROUP_MEMBERSHIPS_PATH: &str = "/2.0/groups/57645/memberships";

    /// ユーザーが所属するグループのAPIのパス
    const USER_MEMBERSHIPS_PATH: &str = "/2.0/users/12345/memberships";

    /// 呼び出すメソッドの種類
    #[derive(Debug, Clone, Copy)]
    enum Target {
        /// グループのメンバー
        Group,

        /// ユーザーが所属するグループ
        User,
    }

    impl Target {
        /// APIのパスを返す。
        fn path(self) -> &'static str {
            match self {
                Self::Group => GROUP_MEMBERSHIPS_PATH,
                Self::User => USER_MEMBERSHIPS_PATH,
            }
        }

        /// メソッドを呼び出して、メンバーシップの1ページを取得する。
        async fn list(
            self,
            client: &BoxClient,
            offset: u64,
            options: &ListGroupMembershipsOptions,
        ) -> BoxClientResult<GroupMembershipsPage> {
            match self {
                Self::Group => {
                    client
                        .list_group_memberships(&GroupId::new("57645"), offset, options)
                        .await
                }
                Self::User => {
                    client
                        .list_user_memberships(&UserId::new("12345"), offset, options)
                        .await
                }
            }
        }
    }

    /// オフセットとオプションをクエリパラメーターで送信し、メンバーシップの1ページをデシリアライズすることを確認
    #[rstest]
    #[case::group_default(Target::Group, 0, ListGroupMembershipsOptions::default(), "offset=0")]
    #[case::group_with_limit(
        Target::Group,
        2,
        ListGroupMembershipsOptions::default().with_limit(100),
        "offset=2&limit=100"
    )]
    #[case::user_default(Target::User, 0, ListGroupMembershipsOptions::default(), "offset=0")]
    #[case::user_with_limit(
        Target::User,
        9998,
        ListGroupMembershipsOptions::default().with_limit(100),
        "offset=9998&limit=100"
    )]
    #[tokio::test]
    async fn test_list_memberships(
        #[case] target: Target,
        #[case] offset: u64,
        #[case] options: ListGroupMembershipsOptions,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(target.path()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [
                    {"type": "group_membership", "id": "1", "role": "member"},
                    {"type": "group_membership", "id": "2", "role": "admin"},
                ],
                "total_count": 5,
                "limit": 100,
                "offset": 2,
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = target.list(&client, offset, &options).await?;

        let ids: Vec<_> = page.entries.iter().map(|m| m.id.as_ref()).collect();
        assert_eq!(ids, vec!["1", "2"]);
        assert_eq!(page.total_count, 5);
        assert_eq!(page.limit, Some(100));
        assert_eq!(page.offset, Some(2));
        assert_eq!(
            request_queries(&server, target.path()).await,
            vec![expected_query]
        );

        Ok(())
    }

    /// 先頭から上限の件数までしか返さないように、上限を超えるメンバーシップをページから取り除くことを確認
    #[rstest]
    #[case::group(Target::Group)]
    #[case::user(Target::User)]
    #[tokio::test]
    async fn test_list_memberships_truncates_at_max(#[case] target: Target) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(target.path()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [
                    {"type": "group_membership", "id": "1"},
                    {"type": "group_membership", "id": "2"},
                ],
                "total_count": 20_000,
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = target
            .list(
                &client,
                MAX_LISTED_GROUP_MEMBERSHIPS - 1,
                &ListGroupMembershipsOptions::default().with_limit(2),
            )
            .await?;

        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.limit, None);
        assert_eq!(page.offset, None);

        Ok(())
    }

    /// オフセットまたは上限が範囲外の場合は、リクエストを送信せずに`InvalidArgument`を返すことを確認
    #[rstest]
    #[case::group_offset_at_max(
        Target::Group,
        MAX_LISTED_GROUP_MEMBERSHIPS,
        ListGroupMembershipsOptions::default()
    )]
    #[case::user_offset_above_max(
        Target::User,
        MAX_LISTED_GROUP_MEMBERSHIPS + 1,
        ListGroupMembershipsOptions::default()
    )]
    #[case::group_zero_limit(Target::Group, 0, ListGroupMembershipsOptions::default().with_limit(0))]
    #[case::user_limit_above_max(
        Target::User,
        0,
        ListGroupMembershipsOptions::default().with_limit(MAX_ITEMS_LIMIT + 1)
    )]
    #[tokio::test]
    async fn test_list_memberships_invalid_arguments(
        #[case] target: Target,
        #[case] offset: u64,
        #[case] options: ListGroupMembershipsOptions,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = target.list(&client, offset, &options).await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }
}

#[cfg(test)]
mod test_list_all_memberships {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// グループのメンバーのAPIのパス
    const GROUP_MEMBERSHIPS_PATH: &str = "/2.0/groups/57645/memberships";

    /// ユーザーが所属するグループのAPIのパス
    const USER_MEMBERSHIPS_PATH: &str = "/2.0/users/12345/memberships";

    /// すべてのメンバーシップを取得するメソッドを呼び出す。
    async fn list_all(
        client: &BoxClient,
        group: bool,
        options: &ListGroupMembershipsOptions,
    ) -> BoxClientResult<Vec<GroupMembership>> {
        if group {
            client
                .list_all_group_memberships(&GroupId::new("57645"), options)
                .await
        } else {
            client
                .list_all_user_memberships(&UserId::new("12345"), options)
                .await
        }
    }

    /// オフセットを進めながらすべてのページを取得し、上限の既定値を各ページで送信することを確認
    #[rstest]
    #[case::group(true, GROUP_MEMBERSHIPS_PATH)]
    #[case::user(false, USER_MEMBERSHIPS_PATH)]
    #[tokio::test]
    async fn test_list_all_memberships(
        #[case] group: bool,
        #[case] request_path: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(request_path))
            .and(query_param("offset", "0"))
            .and(query_param("limit", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [
                    {"type": "group_membership", "id": "1"},
                    {"type": "group_membership", "id": "2"},
                ],
                "total_count": 3,
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(request_path))
            .and(query_param("offset", "2"))
            .and(query_param("limit", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "group_membership", "id": "3"}],
                "total_count": 3,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let memberships = list_all(&client, group, &ListGroupMembershipsOptions::default()).await?;

        let ids: Vec<_> = memberships.iter().map(|m| m.id.as_ref()).collect();
        assert_eq!(ids, vec!["1", "2", "3"]);

        Ok(())
    }

    /// メンバーシップが上限より多い場合は、エラーにせず、先頭から上限の件数までを返すことを確認
    #[rstest]
    #[case::group(true, GROUP_MEMBERSHIPS_PATH)]
    #[case::user(false, USER_MEMBERSHIPS_PATH)]
    #[tokio::test]
    async fn test_list_all_memberships_stops_at_max(
        #[case] group: bool,
        #[case] request_path: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(request_path))
            .respond_with(move |request: &wiremock::Request| {
                let offset: u64 = request
                    .url
                    .query_pairs()
                    .find(|(key, _)| key == "offset")
                    .and_then(|(_, value)| value.parse().ok())
                    .unwrap_or_default();
                let entries: Vec<_> = (offset..offset + 300)
                    .map(|id| json!({"type": "group_membership", "id": id.to_string()}))
                    .collect();
                ResponseTemplate::new(200).set_body_json(json!({
                    "entries": entries,
                    "total_count": 20_000,
                }))
            })
            .expect(34)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let memberships = list_all(
            &client,
            group,
            &ListGroupMembershipsOptions::default().with_limit(300),
        )
        .await?;

        assert_eq!(memberships.len() as u64, MAX_LISTED_GROUP_MEMBERSHIPS);
        assert_eq!(memberships.last().map(|m| m.id.as_ref()), Some("9999"));

        Ok(())
    }
}

#[cfg(test)]
mod test_create_group_membership {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_not_retried, create_client, mount_token, request_count, short_retry,
    };

    /// グループメンバーシップのAPIのパス
    const GROUP_MEMBERSHIPS_PATH: &str = "/2.0/group_memberships";

    /// 作成したメンバーシップのレスポンスを返す。
    fn created_response() -> ResponseTemplate {
        ResponseTemplate::new(201).set_body_json(json!({
            "type": "group_membership",
            "id": "999",
            "role": "member",
        }))
    }

    /// ユーザー、グループおよびオプションをリクエストボディに変換し、作成したメンバーシップを返すことを確認
    #[rstest]
    #[case::minimal(
        CreateGroupMembershipOptions::default(),
        json!({"user": {"id": "12345"}, "group": {"id": "57645"}})
    )]
    #[case::admin_with_permissions(
        CreateGroupMembershipOptions::default()
            .with_role(GroupMembershipRole::Admin)
            .with_configurable_permissions(&[("can_run_reports", false), ("can_instant_login", true)]),
        json!({
            "user": {"id": "12345"},
            "group": {"id": "57645"},
            "role": "admin",
            "configurable_permissions": {"can_run_reports": false, "can_instant_login": true},
        })
    )]
    #[case::member_with_permissions_disabled(
        CreateGroupMembershipOptions::default()
            .with_role(GroupMembershipRole::Member)
            .with_configurable_permissions_disabled(),
        json!({
            "user": {"id": "12345"},
            "group": {"id": "57645"},
            "role": "member",
            "configurable_permissions": null,
        })
    )]
    #[tokio::test]
    async fn test_create_group_membership(
        #[case] options: CreateGroupMembershipOptions<'_>,
        #[case] expected_body: serde_json::Value,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(GROUP_MEMBERSHIPS_PATH))
            .and(body_json(expected_body))
            .respond_with(created_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let membership = client
            .create_group_membership(&UserId::new("12345"), &GroupId::new("57645"), &options)
            .await?;

        assert_eq!(membership.id, GroupMembershipId::new("999"));

        Ok(())
    }

    /// オプションをBOXが拒否する場合は、リクエストを送信せずに`InvalidArgument`を返すことを確認
    #[rstest]
    #[case::unknown_role(
        CreateGroupMembershipOptions::default().with_role(GroupMembershipRole::Unknown)
    )]
    #[case::empty_permission_name(
        CreateGroupMembershipOptions::default().with_configurable_permissions(&[(" ", true)])
    )]
    #[case::duplicate_permission_names(
        CreateGroupMembershipOptions::default()
            .with_configurable_permissions(&[("can_run_reports", true), ("can_run_reports", false)])
    )]
    #[tokio::test]
    async fn test_create_group_membership_invalid_arguments(
        #[case] options: CreateGroupMembershipOptions<'_>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .create_group_membership(&UserId::new("12345"), &GroupId::new("57645"), &options)
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// メンバーシップを重複して作成しないように、5xxを受け取っても再試行しないことを確認
    #[rstest]
    #[case::forbidden(403)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_create_group_membership_error_is_not_retried(
        #[case] status: u16,
    ) -> anyhow::Result<()> {
        assert_not_retried(
            "POST",
            GROUP_MEMBERSHIPS_PATH,
            status,
            async |client: &BoxClient| {
                client
                    .create_group_membership(
                        &UserId::new("12345"),
                        &GroupId::new("57645"),
                        &CreateGroupMembershipOptions::default(),
                    )
                    .await
            },
        )
        .await
    }
}

#[cfg(test)]
mod test_update_group_membership {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_retried_after, create_client, mount_token, request_count, short_retry,
    };

    /// グループメンバーシップのAPIのパス
    const GROUP_MEMBERSHIP_PATH: &str = "/2.0/group_memberships/434534";

    /// 更新したメンバーシップのレスポンスを返す。
    fn updated_response() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "type": "group_membership",
            "id": "434534",
            "role": "admin",
        }))
    }

    /// 変更する属性をリクエストボディに変換し、更新したメンバーシップを返すことを確認
    #[rstest]
    #[case::empty(GroupMembershipUpdate::default(), json!({}))]
    #[case::role_and_permissions(
        GroupMembershipUpdate::default()
            .with_role(GroupMembershipRole::Admin)
            .with_configurable_permissions(&[("can_run_reports", true)]),
        json!({"role": "admin", "configurable_permissions": {"can_run_reports": true}})
    )]
    #[case::permissions_disabled(
        GroupMembershipUpdate::default().with_configurable_permissions_disabled(),
        json!({"configurable_permissions": null})
    )]
    #[tokio::test]
    async fn test_update_group_membership(
        #[case] update: GroupMembershipUpdate<'_>,
        #[case] expected_body: serde_json::Value,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(GROUP_MEMBERSHIP_PATH))
            .and(body_json(expected_body))
            .respond_with(updated_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let membership = client
            .update_group_membership(&GroupMembershipId::new("434534"), &update)
            .await?;

        assert_eq!(membership.id, GroupMembershipId::new("434534"));
        assert_eq!(membership.role, Some(GroupMembershipRole::Admin));

        Ok(())
    }

    /// 変更する属性をBOXが拒否する場合は、リクエストを送信せずに`InvalidArgument`を返すことを確認
    #[rstest]
    #[case::unknown_role(GroupMembershipUpdate::default().with_role(GroupMembershipRole::Unknown))]
    #[case::empty_permission_name(
        GroupMembershipUpdate::default().with_configurable_permissions(&[("", true)])
    )]
    #[tokio::test]
    async fn test_update_group_membership_invalid_arguments(
        #[case] update: GroupMembershipUpdate<'_>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .update_group_membership(&GroupMembershipId::new("434534"), &update)
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// 冪等なリクエストとして、5xxの後に再試行することを確認
    #[tokio::test]
    async fn test_update_group_membership_retries_after_server_error() -> anyhow::Result<()> {
        assert_retried_after(
            "PUT",
            GROUP_MEMBERSHIP_PATH,
            ResponseTemplate::new(500),
            updated_response(),
            async |client: &BoxClient| {
                client
                    .update_group_membership(
                        &GroupMembershipId::new("434534"),
                        &GroupMembershipUpdate::default().with_role(GroupMembershipRole::Admin),
                    )
                    .await
            },
        )
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod test_delete_group_membership {
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

    /// グループメンバーシップのAPIのパス
    const GROUP_MEMBERSHIP_PATH: &str = "/2.0/group_memberships/434534";

    /// `DELETE /group_memberships/{group_membership_id}`を呼び出し、204を受け取ると成功することを確認
    #[tokio::test]
    async fn test_delete_group_membership() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(GROUP_MEMBERSHIP_PATH))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        client
            .delete_group_membership(&GroupMembershipId::new("434534"))
            .await?;

        assert_eq!(
            count_requests(&server, "DELETE", GROUP_MEMBERSHIP_PATH).await,
            1
        );

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 5xxの後に再送すると、削除済みのメンバーシップに対してエラーを受け取るおそれがあるため、
    /// 再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::forbidden(403)]
    #[case::not_found(404)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_delete_group_membership_error_is_not_retried(
        #[case] status: u16,
    ) -> anyhow::Result<()> {
        assert_not_retried(
            "DELETE",
            GROUP_MEMBERSHIP_PATH,
            status,
            async |client: &BoxClient| {
                client
                    .delete_group_membership(&GroupMembershipId::new("434534"))
                    .await
            },
        )
        .await
    }

    /// 429または他の操作によるロックで409を受け取った場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[rstest]
    #[case::too_many_requests(ResponseTemplate::new(429))]
    #[case::operation_blocked_temporary(operation_blocked_response())]
    #[tokio::test]
    async fn test_delete_group_membership_retries_request_not_processed(
        #[case] first_response: ResponseTemplate,
    ) -> anyhow::Result<()> {
        assert_retried_after(
            "DELETE",
            GROUP_MEMBERSHIP_PATH,
            first_response,
            ResponseTemplate::new(204),
            async |client: &BoxClient| {
                client
                    .delete_group_membership(&GroupMembershipId::new("434534"))
                    .await
            },
        )
        .await?;

        Ok(())
    }
}
