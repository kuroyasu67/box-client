//! コラボレーションに関するAPI
//!
//! コラボレーションは、ユーザーまたはグループに、フォルダ、ファイルまたはアプリ項目へのアクセス権限を与える。

use reqwest::{Response, StatusCode};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    AppItemId, BoxClient, BoxClientError, BoxClientResult, BoxItem, CollaborationId, FileId,
    FolderId, GroupId, TermsOfServiceId, UserId,
    api::pagination::{
        MAX_ITEMS_LIMIT, MarkerPage, OffsetPage, collect_all_offset_pages, collect_all_pages,
        marker_page_request, validate_limit,
    },
    request::ApiRequest,
    retry::{AttemptError, Idempotency, read_json},
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

impl CollaborationRole {
    /// リクエストボディに指定する文字列を返す。
    ///
    /// [`CollaborationRole::Unknown`]は、BOXに送信する値を持たないため、`None`を返す。
    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Editor => Some("editor"),
            Self::Viewer => Some("viewer"),
            Self::Previewer => Some("previewer"),
            Self::Uploader => Some("uploader"),
            Self::PreviewerUploader => Some("previewer uploader"),
            Self::ViewerUploader => Some("viewer uploader"),
            Self::CoOwner => Some("co-owner"),
            Self::Owner => Some("owner"),
            Self::Unknown => None,
        }
    }

    /// コラボレーションの作成で、リクエストボディに指定する文字列を返す。
    ///
    /// BOXは、コラボレーションの作成で所有者の役割を受け付けないため、[`CollaborationRole::Owner`]には`None`を返す。
    fn as_create_str(self) -> Option<&'static str> {
        match self {
            Self::Owner => None,
            role => role.as_str(),
        }
    }
}

impl CollaborationStatus {
    /// リクエストボディに指定する文字列を返す。
    ///
    /// [`CollaborationStatus::Unknown`]は、BOXに送信する値を持たないため、`None`を返す。
    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Accepted => Some("accepted"),
            Self::Pending => Some("pending"),
            Self::Rejected => Some("rejected"),
            Self::Unknown => None,
        }
    }
}

/// コラボレーションの対象にするファイルまたはフォルダ
///
/// BOXは、コラボレーションの作成で、ファイルとフォルダだけを対象として受け付ける。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CollaborationItem<'a> {
    /// ファイル
    File(&'a FileId),

    /// フォルダ
    Folder(&'a FolderId),
}

/// コラボレーションでアクセス権限を与えるユーザーまたはグループ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Collaborator<'a> {
    /// ユーザーIDで指定したユーザー
    User(&'a UserId),

    /// メールアドレスで指定したユーザー
    ///
    /// BOXに登録していないメールアドレスを指定した場合、BOXはそのメールアドレスに招待を送信し、
    /// 作成したコラボレーションの[`Collaboration::invite_email`]にメールアドレスを設定する。
    UserLogin(&'a str),

    /// グループIDで指定したグループ
    ///
    /// BOXのドキュメントによると、グループを招待できるかどうかは、グループの設定による。
    Group(&'a GroupId),
}

/// コラボレーションの作成のオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// # Examples
///
/// ```
/// use box_client::CreateCollaborationOptions;
///
/// let options = CreateCollaborationOptions::default()
///     .with_can_view_path(true)
///     .with_notify(false);
/// assert_eq!(options.can_view_path, Some(true));
/// assert_eq!(options.notify, Some(false));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CreateCollaborationOptions {
    /// コラボレーターが、「すべてのファイル」に表示されない項目にアクセスできるようにするかどうか
    ///
    /// `true`の場合、共有した項目は「すべてのファイル」に表示されず、コラボレーターはルートフォルダからのパスを参照できない。
    /// `None`の場合はBOXの既定値を使用する。
    pub is_access_only: Option<bool>,

    /// コラボレーターが、フォルダの親フォルダまでのパスをすべて参照できるようにするかどうか
    ///
    /// コラボレーターは、パスを参照できても、親フォルダの権限を得ない。
    /// BOXのドキュメントによると、フォルダのコラボレーションだけで指定でき、
    /// `true`を指定できるのは所有者または共同所有者だけである。
    /// また、BOXは、この値を`true`にしたコラボレーションを、1人のユーザーあたり1,000件までにすることを推奨している。
    /// `None`の場合はBOXの既定値を使用する。
    pub can_view_path: Option<bool>,

    /// コラボレーションを自動で削除する日時
    ///
    /// BOXのドキュメントによると、管理コンソールのエンタープライズ設定で、招待したコラボレーターを自動で削除する設定のうち、
    /// フォルダの所有者による有効期限の延長を許可する設定を有効にしている場合だけ指定できる。
    /// 設定を有効にしていない場合に指定すると、BOXはエラーを返す。`None`の場合は有効期限を設定しない。
    pub expires_at: Option<OffsetDateTime>,

    /// コラボレーターに電子メールで通知するかどうか
    ///
    /// `None`の場合はBOXの既定値を使用する。
    pub notify: Option<bool>,
}

impl CreateCollaborationOptions {
    /// コラボレーターが、「すべてのファイル」に表示されない項目にアクセスできるようにするかどうかを設定する。
    #[must_use]
    pub fn with_is_access_only(mut self, is_access_only: bool) -> Self {
        self.is_access_only = Some(is_access_only);
        self
    }

    /// コラボレーターが、フォルダの親フォルダまでのパスをすべて参照できるようにするかどうかを設定する。
    ///
    /// フォルダのコラボレーションだけで指定できる。ファイルのコラボレーションで指定した場合は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_can_view_path(mut self, can_view_path: bool) -> Self {
        self.can_view_path = Some(can_view_path);
        self
    }

    /// コラボレーションを自動で削除する日時を設定する。
    #[must_use]
    pub fn with_expires_at(mut self, expires_at: OffsetDateTime) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// コラボレーターに電子メールで通知するかどうかを設定する。
    #[must_use]
    pub fn with_notify(mut self, notify: bool) -> Self {
        self.notify = Some(notify);
        self
    }
}

/// コラボレーションの作成リクエストのボディ
#[derive(Debug, Serialize)]
struct CreateCollaborationBody<'a> {
    /// コラボレーションの対象の項目
    item: CollaborationItemBody<'a>,

    /// アクセス権限を与えるユーザーまたはグループ
    accessible_by: CollaboratorBody<'a>,

    /// 与える役割
    role: &'static str,

    /// コラボレーターが、「すべてのファイル」に表示されない項目にアクセスできるようにするかどうか
    #[serde(skip_serializing_if = "Option::is_none")]
    is_access_only: Option<bool>,

    /// コラボレーターが、フォルダの親フォルダまでのパスをすべて参照できるようにするかどうか
    #[serde(skip_serializing_if = "Option::is_none")]
    can_view_path: Option<bool>,

    /// コラボレーションを自動で削除する日時
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    expires_at: Option<OffsetDateTime>,
}

/// コラボレーションの対象の項目のリクエストボディ
#[derive(Debug, Serialize)]
struct CollaborationItemBody<'a> {
    /// 項目の種類
    #[serde(rename = "type")]
    item_type: &'static str,

    /// 項目のID
    id: &'a str,
}

/// アクセス権限を与えるユーザーまたはグループのリクエストボディ
///
/// BOXは、ユーザーをユーザーIDまたはメールアドレスのどちらかで受け付けるため、指定した方だけを送信する。
#[derive(Debug, Serialize)]
struct CollaboratorBody<'a> {
    /// コラボレーターの種類
    #[serde(rename = "type")]
    collaborator_type: &'static str,

    /// ユーザーまたはグループのID
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,

    /// ユーザーのメールアドレス
    #[serde(skip_serializing_if = "Option::is_none")]
    login: Option<&'a str>,
}

impl<'a> CreateCollaborationBody<'a> {
    /// 引数とオプションからボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// `role`が[`CollaborationRole::Owner`]または[`CollaborationRole::Unknown`]の場合、
    /// [`Collaborator::UserLogin`]のメールアドレスが空文字列または空白だけの場合、
    /// またはファイルのコラボレーションで`can_view_path`を指定した場合は、[`BoxClientError::InvalidArgument`]を返す。
    fn new(
        item: CollaborationItem<'a>,
        collaborator: Collaborator<'a>,
        role: CollaborationRole,
        options: &CreateCollaborationOptions,
    ) -> BoxClientResult<Self> {
        let role = role.as_create_str().ok_or(BoxClientError::InvalidArgument(
            "role of a new collaboration must not be owner or unknown",
        ))?;
        let item = match item {
            CollaborationItem::File(id) => {
                if options.can_view_path.is_some() {
                    return Err(BoxClientError::InvalidArgument(
                        "can_view_path can only be used for folder collaborations",
                    ));
                }
                CollaborationItemBody {
                    item_type: "file",
                    id: id.as_ref(),
                }
            }
            CollaborationItem::Folder(id) => CollaborationItemBody {
                item_type: "folder",
                id: id.as_ref(),
            },
        };
        let accessible_by = match collaborator {
            Collaborator::User(id) => CollaboratorBody {
                collaborator_type: "user",
                id: Some(id.as_ref()),
                login: None,
            },
            Collaborator::UserLogin(login) => {
                if login.trim().is_empty() {
                    return Err(BoxClientError::InvalidArgument(
                        "login of a collaborator must not be empty",
                    ));
                }
                CollaboratorBody {
                    collaborator_type: "user",
                    id: None,
                    login: Some(login),
                }
            }
            Collaborator::Group(id) => CollaboratorBody {
                collaborator_type: "group",
                id: Some(id.as_ref()),
                login: None,
            },
        };

        Ok(Self {
            item,
            accessible_by,
            role,
            is_access_only: options.is_access_only,
            can_view_path: options.can_view_path,
            expires_at: options.expires_at,
        })
    }
}

/// コラボレーションの更新で変更する属性
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// 指定した属性だけを送信し、BOXは指定しない属性を変更しない。
///
/// # Examples
///
/// ```
/// use box_client::{CollaborationRole, CollaborationStatus, CollaborationUpdate};
///
/// // 役割を変更する。
/// let update = CollaborationUpdate::default().with_role(CollaborationRole::Viewer);
/// assert_eq!(update.role, Some(CollaborationRole::Viewer));
///
/// // 保留中の招待を承認する。
/// let update = CollaborationUpdate::default().with_status(CollaborationStatus::Accepted);
/// assert_eq!(update.status, Some(CollaborationStatus::Accepted));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CollaborationUpdate {
    /// 与える役割
    ///
    /// [`CollaborationRole::Owner`]を指定すると、項目の所有者を変更する。この場合、BOXは更新したコラボレーションを削除して
    /// 新しいコラボレーションを作成し、以前の所有者は新しいコラボレーションの共同所有者になる。
    /// [`CollaborationRole::Unknown`]は指定できない。
    pub role: Option<CollaborationRole>,

    /// 保留中の招待の状態
    ///
    /// 状態が[`CollaborationStatus::Pending`]の招待を、[`CollaborationStatus::Accepted`]にすると承認し、
    /// [`CollaborationStatus::Rejected`]にすると拒否する。BOXのドキュメントによると、招待を承認する場合は役割を指定しなくてよい。
    /// [`CollaborationStatus::Unknown`]は指定できない。
    pub status: Option<CollaborationStatus>,

    /// コラボレーションを自動で削除する日時
    ///
    /// BOXのドキュメントによると、管理コンソールのエンタープライズ設定で、招待したコラボレーターを自動で削除する設定のうち、
    /// フォルダの所有者による有効期限の延長を許可する設定を有効にしている場合だけ指定できる。
    /// また、その設定を有効にした後に作成したコラボレーションだけに有効期限を設定できる。
    /// 設定を有効にしていない場合に指定すると、BOXは403を返す。
    pub expires_at: Option<OffsetDateTime>,

    /// コラボレーターが、フォルダの親フォルダまでのパスをすべて参照できるようにするかどうか
    ///
    /// BOXのドキュメントによると、フォルダのコラボレーションだけで指定でき、既存のコラボレーションで変更できるのは所有者だけである。
    pub can_view_path: Option<bool>,
}

impl CollaborationUpdate {
    /// 与える役割を設定する。
    ///
    /// [`CollaborationRole::Owner`]を指定すると、項目の所有者を変更する。
    #[must_use]
    pub fn with_role(mut self, role: CollaborationRole) -> Self {
        self.role = Some(role);
        self
    }

    /// 保留中の招待の状態を設定する。
    #[must_use]
    pub fn with_status(mut self, status: CollaborationStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// コラボレーションを自動で削除する日時を設定する。
    #[must_use]
    pub fn with_expires_at(mut self, expires_at: OffsetDateTime) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// コラボレーターが、フォルダの親フォルダまでのパスをすべて参照できるようにするかどうかを設定する。
    #[must_use]
    pub fn with_can_view_path(mut self, can_view_path: bool) -> Self {
        self.can_view_path = Some(can_view_path);
        self
    }
}

/// コラボレーションの更新の結果
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CollaborationUpdateOutcome {
    /// コラボレーションを更新した。
    Updated(Box<Collaboration>),

    /// 項目の所有者を変更した。
    ///
    /// BOXは、更新したコラボレーションを削除して新しいコラボレーションを作成し、更新後のコラボレーションを返さない。
    /// 新しいコラボレーションは、[`BoxClient::list_file_collaborations`]または
    /// [`BoxClient::list_folder_collaborations`]で確認する。
    OwnerChanged,
}

/// コラボレーションの更新リクエストのボディ
#[derive(Debug, Serialize)]
struct UpdateCollaborationBody {
    /// 与える役割
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,

    /// 保留中の招待の状態
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'static str>,

    /// コラボレーションを自動で削除する日時
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    expires_at: Option<OffsetDateTime>,

    /// コラボレーターが、フォルダの親フォルダまでのパスをすべて参照できるようにするかどうか
    #[serde(skip_serializing_if = "Option::is_none")]
    can_view_path: Option<bool>,
}

/// コラボレーションの更新の成功レスポンスを読み込む。
///
/// 所有者を変更した場合、BOXはボディのない204を返すため、[`CollaborationUpdateOutcome::OwnerChanged`]を返す。
/// それ以外の場合は、更新したコラボレーションを読み込んで[`CollaborationUpdateOutcome::Updated`]を返す。
async fn read_update_outcome(
    response: Response,
    idempotency: Idempotency,
) -> Result<CollaborationUpdateOutcome, AttemptError> {
    if response.status() == StatusCode::NO_CONTENT {
        return Ok(CollaborationUpdateOutcome::OwnerChanged);
    }
    read_json(response, idempotency)
        .await
        .map(|collaboration| CollaborationUpdateOutcome::Updated(Box::new(collaboration)))
}

impl UpdateCollaborationBody {
    /// 変更する属性からボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// `update.role`が[`CollaborationRole::Unknown`]の場合、または`update.status`が[`CollaborationStatus::Unknown`]の場合は、
    /// [`BoxClientError::InvalidArgument`]を返す。
    fn new(update: &CollaborationUpdate) -> BoxClientResult<Self> {
        let role = update
            .role
            .map(|role| {
                role.as_str().ok_or(BoxClientError::InvalidArgument(
                    "role of a collaboration must not be unknown",
                ))
            })
            .transpose()?;
        let status = update
            .status
            .map(|status| {
                status.as_str().ok_or(BoxClientError::InvalidArgument(
                    "status of a collaboration must not be unknown",
                ))
            })
            .transpose()?;

        Ok(Self {
            role,
            status,
            expires_at: update.expires_at,
            can_view_path: update.can_view_path,
        })
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

/// オフセット方式でコラボレーションをリストするAPIで指定できるオフセットの最大値を表す整数リテラルに展開する。
///
/// `concat!`は定数を受け付けないため、[`MAX_COLLABORATIONS_OFFSET`]とエラーのメッセージで、このマクロを共有する。
/// これにより、最大値を変更したときに、メッセージの値も追随させる。
macro_rules! max_collaborations_offset {
    () => {
        10_000
    };
}

/// オフセット方式でコラボレーションをリストするAPIで指定できるオフセットの最大値
///
/// グループのコラボレーションのリストおよび保留中のコラボレーションの招待のリストで共有する。
/// BOXは、どちらのAPIでも、オフセットがこの値を超えるリクエストを400で拒否する。
pub const MAX_COLLABORATIONS_OFFSET: u64 = max_collaborations_offset!();

/// オフセットがBOXの受け付ける最大値以下であることを検証する。
///
/// # Errors
///
/// `offset`が[`MAX_COLLABORATIONS_OFFSET`]を超える場合は、[`BoxClientError::InvalidArgument`]を返す。
fn validate_collaborations_offset(offset: u64) -> BoxClientResult<u64> {
    if MAX_COLLABORATIONS_OFFSET < offset {
        return Err(BoxClientError::InvalidArgument(concat!(
            "offset must be less than or equal to ",
            max_collaborations_offset!()
        )));
    }
    Ok(offset)
}

/// グループのコラボレーションをリストするときのオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// # Examples
///
/// ```
/// use box_client::ListGroupCollaborationsOptions;
///
/// let options = ListGroupCollaborationsOptions::default().with_limit(100);
/// assert_eq!(options.limit, Some(100));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListGroupCollaborationsOptions {
    /// 1ページで取得するコラボレーション数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,
}

impl ListGroupCollaborationsOptions {
    /// 1ページで取得するコラボレーション数の上限を設定する。
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。範囲外の値は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// 保留中のコラボレーションの招待をリストするときのオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// # Examples
///
/// ```
/// use box_client::ListPendingCollaborationsOptions;
///
/// let options = ListPendingCollaborationsOptions::default().with_limit(100);
/// assert_eq!(options.limit, Some(100));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListPendingCollaborationsOptions {
    /// 1ページで取得するコラボレーション数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,
}

impl ListPendingCollaborationsOptions {
    /// 1ページで取得するコラボレーション数の上限を設定する。
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。範囲外の値は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// オフセット方式でページ送りするコラボレーションの1ページ
///
/// BOXは、フォルダおよびファイルのコラボレーションと異なり、グループのコラボレーションのリストおよび
/// 保留中のコラボレーションの招待のリストをオフセット方式でページ送りする。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct CollaborationsOffsetPage {
    /// このページのコラボレーション
    pub entries: Vec<Collaboration>,

    /// 最後のコラボレーションのオフセットに1を加えた値
    pub total_count: u64,

    /// このページの取得に使用したコラボレーション数の上限
    pub limit: u32,

    /// このページの最初のコラボレーションのオフセット（0始まり）
    pub offset: u64,
}

impl OffsetPage for CollaborationsOffsetPage {
    type Entry = Collaboration;

    fn into_parts(self) -> (Vec<Collaboration>, u64) {
        (self.entries, self.total_count)
    }
}

/// フォルダまたはファイルのコラボレーションの1ページ
///
/// BOXは、フォルダおよびファイルのコラボレーションのリストをマーカー方式でページ送りする。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct CollaborationsPage {
    /// このページのコラボレーション
    pub entries: Vec<Collaboration>,

    /// このページの取得に使用したコラボレーション数の上限
    ///
    /// BOXのドキュメントには記載があるが、実際のBOXはレスポンスに含めない場合があるため、その場合は`None`になる。
    pub limit: Option<u32>,

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

    /// ファイルまたはフォルダに、ユーザーまたはグループのコラボレーションを作成する。
    ///
    /// HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。
    /// 5xxまたはタイムアウトの後に再送すると、同じコラボレーションを重複して作成するおそれがあるためである。
    ///
    /// # Arguments
    ///
    /// - `item`: コラボレーションの対象にするファイルまたはフォルダ
    /// - `collaborator`: アクセス権限を与えるユーザーまたはグループ
    /// - `role`: 与える役割。[`CollaborationRole::Owner`]および[`CollaborationRole::Unknown`]は指定できない。
    /// - `options`: 「すべてのファイル」への表示、親フォルダのパスの参照、有効期限および通知のオプション
    ///
    /// # Errors
    ///
    /// `role`が[`CollaborationRole::Owner`]もしくは[`CollaborationRole::Unknown`]の場合、
    /// [`Collaborator::UserLogin`]のメールアドレスが空文字列もしくは空白だけの場合、
    /// またはファイルのコラボレーションで`options.can_view_path`を指定した場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 例えば、情報バリアによってコラボレーションの作成を禁止している場合は、BOXはエラーコードが
    /// `forbidden_by_policy`の403を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがコラボレーションを作成した可能性がある。
    /// したがって、[`BoxClient::list_file_collaborations`]または[`BoxClient::list_folder_collaborations`]で
    /// 項目のコラボレーションを確認する。
    pub async fn create_collaboration(
        &self,
        item: CollaborationItem<'_>,
        collaborator: Collaborator<'_>,
        role: CollaborationRole,
        options: &CreateCollaborationOptions,
    ) -> BoxClientResult<Collaboration> {
        let body = CreateCollaborationBody::new(item, collaborator, role, options)?;
        let mut request = ApiRequest::post(["collaborations"]);
        if let Some(notify) = options.notify {
            request = request.query("notify", if notify { "true" } else { "false" });
        }
        let request = request.json(&body)?;

        let collaboration: Collaboration = self.send_json(request).await?;

        // メールアドレスは個人情報であるため、ログに記録しない。
        tracing::debug!(
            collaboration_id = collaboration.id.as_ref(),
            item_type = body.item.item_type,
            item_id = body.item.id,
            "created collaboration"
        );

        Ok(collaboration)
    }

    /// コラボレーションを更新する。
    ///
    /// 役割、保留中の招待の状態、有効期限、および親フォルダのパスを参照できるかどうかを変更できる。
    /// 役割に[`CollaborationRole::Owner`]を指定すると項目の所有者を変更し、
    /// 状態に[`CollaborationStatus::Accepted`]または[`CollaborationStatus::Rejected`]を指定すると保留中の招待を承認または拒否する。
    ///
    /// 所有者を変更する場合は、BOXが更新したコラボレーションを削除するため、冪等でないリクエストとして送信する。
    /// 5xxまたはタイムアウトの後に再送すると、コラボレーションがすでに存在せず、BOXが404を返すおそれがあるためである。
    /// それ以外の更新は、同じ値を何度設定しても結果が変わらないため、冪等なリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `collaboration_id`: 更新するコラボレーションのID
    /// - `update`: 変更する属性
    ///
    /// # Returns
    ///
    /// コラボレーションを更新した場合は、更新したコラボレーションを[`CollaborationUpdateOutcome::Updated`]で返す。
    /// 所有者を変更した場合は、BOXが204を返すため、[`CollaborationUpdateOutcome::OwnerChanged`]を返す。
    ///
    /// # Errors
    ///
    /// `update.role`が[`CollaborationRole::Unknown`]の場合、または`update.status`が[`CollaborationStatus::Unknown`]の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// コラボレーションを更新する権限がない場合、または有効期限を設定できない場合は、BOXが403を返すため、
    /// [`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 所有者の変更が5xxまたはタイムアウトでエラーを返した場合は、BOXが所有者を変更した可能性がある。
    /// したがって、[`BoxClient::list_file_collaborations`]または[`BoxClient::list_folder_collaborations`]で
    /// 項目のコラボレーションを確認する。
    pub async fn update_collaboration(
        &self,
        collaboration_id: &CollaborationId,
        update: &CollaborationUpdate,
    ) -> BoxClientResult<CollaborationUpdateOutcome> {
        let body = UpdateCollaborationBody::new(update)?;
        let mut request = ApiRequest::put(["collaborations", collaboration_id.as_ref()]);
        if update.role == Some(CollaborationRole::Owner) {
            request = request.idempotency(Idempotency::NonIdempotent);
        }
        let request = request.json(&body)?;

        let outcome = self.send(request, read_update_outcome).await?;

        tracing::debug!(
            collaboration_id = collaboration_id.as_ref(),
            owner_changed = matches!(outcome, CollaborationUpdateOutcome::OwnerChanged),
            "updated collaboration"
        );

        Ok(outcome)
    }

    /// コラボレーションを削除する。
    ///
    /// コラボレーターは、コラボレーションの対象の項目にアクセスできなくなる。
    ///
    /// BOXクライアントは、冪等なリクエストであれば、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがコラボレーションの削除を完了している可能性がある。
    /// BOXが削除した後にリクエストを再送すると、削除済みのコラボレーションに対する削除となるため、BOXはエラーを返しうる。
    /// その結果、削除に成功したにもかかわらず、削除に失敗したと誤って判断するおそれがある。
    /// したがって、HTTPメソッドがDELETEであっても冪等でないリクエストとして送信し、
    /// BOXがリクエストを処理していないことが明らかな429、エラーコードが`operation_blocked_temporary`の409
    /// および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `collaboration_id`: 削除するコラボレーションのID
    ///
    /// # Errors
    ///
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがコラボレーションを削除した可能性があるため、
    /// [`BoxClient::collaboration_information`]でコラボレーションが存在するか確認する。
    pub async fn delete_collaboration(
        &self,
        collaboration_id: &CollaborationId,
    ) -> BoxClientResult<()> {
        let request = ApiRequest::delete(["collaborations", collaboration_id.as_ref()])
            .idempotency(Idempotency::NonIdempotent);

        self.send_empty(request).await?;

        tracing::debug!(
            collaboration_id = collaboration_id.as_ref(),
            "deleted collaboration"
        );

        Ok(())
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

    /// グループのコラボレーションを1ページ分リストする。
    ///
    /// グループにアクセス権限を与えたコラボレーションを返す。各コラボレーションから、
    /// グループがアクセスできる項目と役割を確認できる。グループにコラボレーションがない場合は、空のページを返す。
    /// BOXのドキュメントによると、エンタープライズのグループを調べるには、ユーザーが管理者の権限を持つ必要がある。
    ///
    /// 1回の呼び出しで返すコラボレーションは最大で`limit`件である。次のページを取得する場合は、
    /// 前のページの`offset`に、前のページの`entries`の数を加えた値を`offset`に指定して再度呼び出す。
    /// すべてのコラボレーションが必要な場合は、[`BoxClient::list_all_group_collaborations`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `group_id`: コラボレーションをリストするグループのID
    /// - `offset`: 取得する最初のコラボレーションのオフセット（0始まり）。[`MAX_COLLABORATIONS_OFFSET`]以下を指定する。
    /// - `options`: コラボレーション数の上限などのオプション
    ///
    /// # Errors
    ///
    /// `offset`が[`MAX_COLLABORATIONS_OFFSET`]を超える場合、または`options.limit`が範囲外の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// グループが存在しない場合、またはグループを調べる権限がない場合は、
    /// [`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_group_collaborations(
        &self,
        group_id: &GroupId,
        offset: u64,
        options: &ListGroupCollaborationsOptions,
    ) -> BoxClientResult<CollaborationsOffsetPage> {
        let mut request = ApiRequest::get(["groups", group_id.as_ref(), "collaborations"]).query(
            "offset",
            validate_collaborations_offset(offset)?.to_string(),
        );
        if let Some(limit) = options.limit {
            request = request.query("limit", validate_limit(limit)?.to_string());
        }

        self.send_json(request).await
    }

    /// グループのすべてのコラボレーションを、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。
    /// 取得したコラボレーションの数が`total_count`に達した場合、またはBOXが空のページを返した場合に終了する。
    ///
    /// # Arguments
    ///
    /// - `group_id`: コラボレーションをリストするグループのID
    /// - `options`: コラボレーション数の上限などのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// 次のページのオフセットが[`MAX_COLLABORATIONS_OFFSET`]を超え、残りのコラボレーションを取得できない場合は、
    /// 一部のコラボレーションだけを返さないように[`BoxClientError::UnexpectedResponse`](crate::BoxClientError::UnexpectedResponse)を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みのコラボレーションを破棄してエラーを返す。
    pub async fn list_all_group_collaborations(
        &self,
        group_id: &GroupId,
        options: &ListGroupCollaborationsOptions,
    ) -> BoxClientResult<Vec<Collaboration>> {
        let options = ListGroupCollaborationsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
        };
        let (collaborations, pages) = collect_all_offset_pages(
            MAX_COLLABORATIONS_OFFSET,
            "group collaborations cannot be listed beyond the maximum offset",
            async |offset| {
                self.list_group_collaborations(group_id, offset, &options)
                    .await
            },
        )
        .await?;

        tracing::debug!(
            group_id = group_id.as_ref(),
            pages,
            collaborations = collaborations.len(),
            "listed all collaborations of group"
        );

        Ok(collaborations)
    }

    /// 認証中のユーザー宛ての、保留中のコラボレーションの招待を1ページ分リストする。
    ///
    /// 状態が[`CollaborationStatus::Pending`]のコラボレーションを返す。BOXは、保留中のコラボレーションの
    /// 対象の項目（[`Collaboration::item`]）を`None`で返す。保留中の招待がない場合は、空のページを返す。
    /// JWT認証でエンタープライズとして認証している場合は、アプリケーションのサービスアカウント宛ての招待を返す。
    ///
    /// 1回の呼び出しで返すコラボレーションは最大で`limit`件である。次のページを取得する場合は、
    /// 前のページの`offset`に、前のページの`entries`の数を加えた値を`offset`に指定して再度呼び出す。
    /// すべての招待が必要な場合は、[`BoxClient::list_all_pending_collaborations`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `offset`: 取得する最初のコラボレーションのオフセット（0始まり）。[`MAX_COLLABORATIONS_OFFSET`]以下を指定する。
    /// - `options`: コラボレーション数の上限などのオプション
    ///
    /// # Errors
    ///
    /// `offset`が[`MAX_COLLABORATIONS_OFFSET`]を超える場合、または`options.limit`が範囲外の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`](crate::BoxClientError::Api)を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_pending_collaborations(
        &self,
        offset: u64,
        options: &ListPendingCollaborationsOptions,
    ) -> BoxClientResult<CollaborationsOffsetPage> {
        // BOXは`status`を必須とし、`pending`だけを受け付ける。
        let mut request = ApiRequest::get(["collaborations"])
            .query("status", "pending")
            .query(
                "offset",
                validate_collaborations_offset(offset)?.to_string(),
            );
        if let Some(limit) = options.limit {
            request = request.query("limit", validate_limit(limit)?.to_string());
        }

        self.send_json(request).await
    }

    /// 認証中のユーザー宛ての、保留中のすべてのコラボレーションの招待を、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。
    /// 取得したコラボレーションの数が`total_count`に達した場合、またはBOXが空のページを返した場合に終了する。
    ///
    /// # Arguments
    ///
    /// - `options`: コラボレーション数の上限などのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`](crate::BoxClientError::InvalidArgument)を返す。
    /// 次のページのオフセットが[`MAX_COLLABORATIONS_OFFSET`]を超え、残りの招待を取得できない場合は、
    /// 一部の招待だけを返さないように[`BoxClientError::UnexpectedResponse`](crate::BoxClientError::UnexpectedResponse)を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みの招待を破棄してエラーを返す。
    pub async fn list_all_pending_collaborations(
        &self,
        options: &ListPendingCollaborationsOptions,
    ) -> BoxClientResult<Vec<Collaboration>> {
        let options = ListPendingCollaborationsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
        };
        let (collaborations, pages) = collect_all_offset_pages(
            MAX_COLLABORATIONS_OFFSET,
            "pending collaborations cannot be listed beyond the maximum offset",
            async |offset| self.list_pending_collaborations(offset, &options).await,
        )
        .await?;

        tracing::debug!(
            pages,
            collaborations = collaborations.len(),
            "listed all pending collaborations"
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
mod test_create_collaboration {
    use rstest::rstest;
    use serde_json::json;
    use time::macros::datetime;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_not_retried, assert_not_retried_with_response, create_client, mount_token,
        request_count, request_queries, short_retry,
    };

    /// コラボレーションのAPIのパス
    const COLLABORATIONS_PATH: &str = "/2.0/collaborations";

    /// テストで使用するファイルID
    fn file_id() -> FileId {
        FileId::new("11446498")
    }

    /// テストで使用するフォルダID
    fn folder_id() -> FolderId {
        FolderId::new("12345")
    }

    /// テストで使用するユーザーID
    fn user_id() -> UserId {
        UserId::new("33224412")
    }

    /// テストで使用するグループID
    fn group_id() -> GroupId {
        GroupId::new("57645")
    }

    /// 作成したコラボレーションのレスポンスを返す。
    fn created_response() -> ResponseTemplate {
        ResponseTemplate::new(201).set_body_json(json!({
            "type": "collaboration",
            "id": "999",
            "role": "editor",
            "status": "pending",
        }))
    }

    /// 項目、コラボレーター、役割およびオプションをリクエストボディとクエリパラメーターに変換し、
    /// 作成したコラボレーションを返すことを確認
    #[rstest]
    // ファイルをメールアドレスで指定したユーザーと共有する最小限の指定
    #[case::file_login(
        "file",
        "login",
        CollaborationRole::Editor,
        CreateCollaborationOptions::default(),
        json!({
            "item": {"type": "file", "id": "11446498"},
            "accessible_by": {"type": "user", "login": "user@example.com"},
            "role": "editor",
        }),
        ""
    )]
    // フォルダをユーザーIDで指定したユーザーと共有し、すべてのオプションを指定する
    #[case::folder_user_all_options(
        "folder",
        "user",
        CollaborationRole::ViewerUploader,
        CreateCollaborationOptions::default()
            .with_is_access_only(true)
            .with_can_view_path(true)
            .with_expires_at(datetime!(2026-12-31 23:59:00 -07:00))
            .with_notify(false),
        json!({
            "item": {"type": "folder", "id": "12345"},
            "accessible_by": {"type": "user", "id": "33224412"},
            "role": "viewer uploader",
            "is_access_only": true,
            "can_view_path": true,
            "expires_at": "2026-12-31T23:59:00-07:00",
        }),
        "notify=false"
    )]
    // フォルダをグループと共有する
    #[case::folder_group(
        "folder",
        "group",
        CollaborationRole::CoOwner,
        CreateCollaborationOptions::default().with_notify(true),
        json!({
            "item": {"type": "folder", "id": "12345"},
            "accessible_by": {"type": "group", "id": "57645"},
            "role": "co-owner",
        }),
        "notify=true"
    )]
    #[tokio::test]
    async fn test_create_collaboration(
        #[case] item_kind: &str,
        #[case] collaborator_kind: &str,
        #[case] role: CollaborationRole,
        #[case] options: CreateCollaborationOptions,
        #[case] expected_body: serde_json::Value,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(COLLABORATIONS_PATH))
            .and(body_json(expected_body))
            .respond_with(created_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;
        let (file_id, folder_id, user_id, group_id) =
            (file_id(), folder_id(), user_id(), group_id());
        let item = match item_kind {
            "file" => CollaborationItem::File(&file_id),
            _ => CollaborationItem::Folder(&folder_id),
        };
        let collaborator = match collaborator_kind {
            "user" => Collaborator::User(&user_id),
            "group" => Collaborator::Group(&group_id),
            _ => Collaborator::UserLogin("user@example.com"),
        };

        let collaboration = client
            .create_collaboration(item, collaborator, role, &options)
            .await?;

        assert_eq!(collaboration.id, CollaborationId::new("999"));
        assert_eq!(collaboration.status, Some(CollaborationStatus::Pending));
        assert_eq!(
            request_queries(&server, COLLABORATIONS_PATH).await,
            vec![expected_query]
        );

        Ok(())
    }

    /// BOXが受け付けない役割、空のメールアドレス、またはファイルでの`can_view_path`を指定した場合は、
    /// リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::owner("folder", Some("user@example.com"), CollaborationRole::Owner, None)]
    #[case::unknown("folder", Some("user@example.com"), CollaborationRole::Unknown, None)]
    #[case::empty_login("folder", Some(""), CollaborationRole::Editor, None)]
    #[case::blank_login("folder", Some("  "), CollaborationRole::Editor, None)]
    #[case::can_view_path_on_file("file", None, CollaborationRole::Editor, Some(true))]
    #[case::can_view_path_false_on_file("file", None, CollaborationRole::Editor, Some(false))]
    #[tokio::test]
    async fn test_create_collaboration_rejects_without_request(
        #[case] item_kind: &str,
        #[case] login: Option<&str>,
        #[case] role: CollaborationRole,
        #[case] can_view_path: Option<bool>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;
        let (file_id, folder_id, user_id) = (file_id(), folder_id(), user_id());
        let item = match item_kind {
            "file" => CollaborationItem::File(&file_id),
            _ => CollaborationItem::Folder(&folder_id),
        };
        let collaborator = login.map_or(Collaborator::User(&user_id), Collaborator::UserLogin);
        let options = CreateCollaborationOptions {
            can_view_path,
            ..CreateCollaborationOptions::default()
        };

        let result = client
            .create_collaboration(item, collaborator, role, &options)
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// 作成は冪等でないため、5xxを再試行せずにそのまま返すことを確認
    #[tokio::test]
    async fn test_create_collaboration_error_is_not_retried() -> anyhow::Result<()> {
        let folder_id = folder_id();
        let user_id = user_id();
        assert_not_retried(
            "POST",
            COLLABORATIONS_PATH,
            500,
            async |client: &BoxClient| {
                client
                    .create_collaboration(
                        CollaborationItem::Folder(&folder_id),
                        Collaborator::User(&user_id),
                        CollaborationRole::Editor,
                        &CreateCollaborationOptions::default(),
                    )
                    .await
            },
        )
        .await
    }

    /// 情報バリアによって作成を禁止している場合は、BOXのエラーコードを含むエラーを返すことを確認
    #[tokio::test]
    async fn test_create_collaboration_forbidden_by_policy() -> anyhow::Result<()> {
        let folder_id = folder_id();
        let user_id = user_id();
        let error = assert_not_retried_with_response(
            "POST",
            COLLABORATIONS_PATH,
            ResponseTemplate::new(403).set_body_json(json!({
                "type": "error",
                "status": 403,
                "code": "forbidden_by_policy",
                "message": "Creating a collaboration is forbidden due to information barrier restrictions.",
            })),
            async |client: &BoxClient| {
                client
                    .create_collaboration(
                        CollaborationItem::Folder(&folder_id),
                        Collaborator::User(&user_id),
                        CollaborationRole::Editor,
                        &CreateCollaborationOptions::default(),
                    )
                    .await
            },
        )
        .await?;

        assert_eq!(error.status, 403);
        assert_eq!(error.code.as_deref(), Some("forbidden_by_policy"));

        Ok(())
    }
}

#[cfg(test)]
mod test_update_collaboration {
    use rstest::rstest;
    use serde_json::json;
    use time::macros::datetime;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_not_retried, assert_retried_after, create_client, mount_token, request_count,
        short_retry,
    };

    /// コラボレーションのAPIのパス
    const COLLABORATION_PATH: &str = "/2.0/collaborations/1234";

    /// 更新したコラボレーションのレスポンスを返す。
    fn updated_response() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "type": "collaboration",
            "id": "1234",
            "role": "viewer",
            "status": "accepted",
        }))
    }

    /// 指定した属性だけをリクエストボディで送信し、更新したコラボレーションを返すことを確認
    #[rstest]
    #[case::role(
        CollaborationUpdate::default().with_role(CollaborationRole::Viewer),
        json!({"role": "viewer"})
    )]
    #[case::accept(
        CollaborationUpdate::default().with_status(CollaborationStatus::Accepted),
        json!({"status": "accepted"})
    )]
    #[case::reject(
        CollaborationUpdate::default().with_status(CollaborationStatus::Rejected),
        json!({"status": "rejected"})
    )]
    #[case::all(
        CollaborationUpdate::default()
            .with_role(CollaborationRole::PreviewerUploader)
            .with_status(CollaborationStatus::Accepted)
            .with_expires_at(datetime!(2026-12-31 23:59:00 -07:00))
            .with_can_view_path(false),
        json!({
            "role": "previewer uploader",
            "status": "accepted",
            "expires_at": "2026-12-31T23:59:00-07:00",
            "can_view_path": false,
        })
    )]
    #[tokio::test]
    async fn test_update_collaboration(
        #[case] update: CollaborationUpdate,
        #[case] expected_body: serde_json::Value,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(COLLABORATION_PATH))
            .and(body_json(expected_body))
            .respond_with(updated_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let outcome = client
            .update_collaboration(&CollaborationId::new("1234"), &update)
            .await?;

        let CollaborationUpdateOutcome::Updated(collaboration) = outcome else {
            anyhow::bail!("unexpected outcome: {outcome:?}");
        };
        assert_eq!(collaboration.id, CollaborationId::new("1234"));
        assert_eq!(collaboration.role, Some(CollaborationRole::Viewer));

        Ok(())
    }

    /// 所有者を変更し、BOXが204を返した場合は、所有者を変更したことを返すことを確認
    #[tokio::test]
    async fn test_update_collaboration_owner_changed() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(COLLABORATION_PATH))
            .and(body_json(json!({"role": "owner"})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let outcome = client
            .update_collaboration(
                &CollaborationId::new("1234"),
                &CollaborationUpdate::default().with_role(CollaborationRole::Owner),
            )
            .await?;

        assert_eq!(outcome, CollaborationUpdateOutcome::OwnerChanged);

        Ok(())
    }

    /// 役割または状態に`Unknown`を指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::unknown_role(CollaborationUpdate::default().with_role(CollaborationRole::Unknown))]
    #[case::unknown_status(CollaborationUpdate::default().with_status(CollaborationStatus::Unknown))]
    #[tokio::test]
    async fn test_update_collaboration_rejects_without_request(
        #[case] update: CollaborationUpdate,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .update_collaboration(&CollaborationId::new("1234"), &update)
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// 所有者の変更は冪等でないため、5xxを再試行せずにそのまま返すことを確認
    #[tokio::test]
    async fn test_update_collaboration_owner_change_is_not_retried() -> anyhow::Result<()> {
        assert_not_retried(
            "PUT",
            COLLABORATION_PATH,
            500,
            async |client: &BoxClient| {
                client
                    .update_collaboration(
                        &CollaborationId::new("1234"),
                        &CollaborationUpdate::default().with_role(CollaborationRole::Owner),
                    )
                    .await
            },
        )
        .await
    }

    /// 所有者の変更以外は冪等であるため、5xxの後に再試行することを確認
    #[tokio::test]
    async fn test_update_collaboration_role_change_is_retried() -> anyhow::Result<()> {
        // `assert_retried_after`は、リクエストを2回送信したことも確認する。
        assert_retried_after(
            "PUT",
            COLLABORATION_PATH,
            ResponseTemplate::new(500),
            updated_response(),
            async |client: &BoxClient| {
                client
                    .update_collaboration(
                        &CollaborationId::new("1234"),
                        &CollaborationUpdate::default().with_role(CollaborationRole::Viewer),
                    )
                    .await
            },
        )
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod test_delete_collaboration {
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

    /// コラボレーションのAPIのパス
    const COLLABORATION_PATH: &str = "/2.0/collaborations/1234";

    /// `DELETE /collaborations/{collaboration_id}`を呼び出し、204を受け取ると成功することを確認
    #[tokio::test]
    async fn test_delete_collaboration() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(COLLABORATION_PATH))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        client
            .delete_collaboration(&CollaborationId::new("1234"))
            .await?;

        assert_eq!(
            count_requests(&server, "DELETE", COLLABORATION_PATH).await,
            1
        );

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 5xxの後に再送すると、削除済みのコラボレーションに対してエラーを受け取るおそれがあるため、
    /// 再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::forbidden(403)]
    #[case::not_found(404)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_delete_collaboration_error_is_not_retried(
        #[case] status: u16,
    ) -> anyhow::Result<()> {
        assert_not_retried(
            "DELETE",
            COLLABORATION_PATH,
            status,
            async |client: &BoxClient| {
                client
                    .delete_collaboration(&CollaborationId::new("1234"))
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
    async fn test_delete_collaboration_retries_request_not_processed(
        #[case] first_response: ResponseTemplate,
    ) -> anyhow::Result<()> {
        assert_retried_after(
            "DELETE",
            COLLABORATION_PATH,
            first_response,
            ResponseTemplate::new(204),
            async |client: &BoxClient| {
                client
                    .delete_collaboration(&CollaborationId::new("1234"))
                    .await
            },
        )
        .await?;

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
        assert_eq!(page.limit, Some(100));
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

    /// BOXが`limit`を含めずに返したレスポンスでも、`limit`を`None`としてページをデシリアライズすることを確認
    #[tokio::test]
    async fn test_list_folder_collaborations_without_limit() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [
                    {"type": "collaboration", "id": "1", "role": "editor", "status": "accepted"},
                ],
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

        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.limit, None);
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
mod test_list_group_collaborations {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, Request, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    use super::*;
    use crate::test_support::{
        count_requests_to_path, create_client, mount_token, not_found_response, request_count,
        short_retry,
    };

    /// グループのコラボレーションのAPIのパス
    const COLLABORATIONS_PATH: &str = "/2.0/groups/789/collaborations";

    /// コラボレーションのJSONを返す。
    fn collaboration_json(id: u64) -> serde_json::Value {
        json!({
            "type": "collaboration",
            "id": id.to_string(),
            "accessible_by": {"type": "group", "id": "789", "name": "Support"},
            "role": "viewer",
            "status": "accepted",
        })
    }

    /// `offset`から`limit`件のコラボレーションを、全体が`total`件として返すレスポンスを作成する。
    fn page_response(request: &Request, total: u64) -> ResponseTemplate {
        let param = |name: &str| {
            request
                .url
                .query_pairs()
                .find(|(key, _)| key == name)
                .and_then(|(_, value)| value.parse::<u64>().ok())
        };
        let offset = param("offset").unwrap_or(0);
        let limit = param("limit").unwrap_or(1000);
        let end = total.min(offset + limit);
        let entries: Vec<_> = (offset..end).map(collaboration_json).collect();
        ResponseTemplate::new(200).set_body_json(json!({
            "total_count": total,
            "limit": limit,
            "offset": offset,
            "entries": entries,
        }))
    }

    /// `offset`と`limit`をクエリパラメーターで送信し、コラボレーションの1ページをデシリアライズすることを確認
    #[tokio::test]
    async fn test_list_group_collaborations() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .and(query_param("offset", "2"))
            .and(query_param("limit", "2"))
            .respond_with(|request: &Request| page_response(request, 5))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client
            .list_group_collaborations(
                &GroupId::new("789"),
                2,
                &ListGroupCollaborationsOptions::default().with_limit(2),
            )
            .await?;

        assert_eq!(page.total_count, 5);
        assert_eq!(page.limit, 2);
        assert_eq!(page.offset, 2);
        let ids: Vec<_> = page.entries.iter().map(|c| c.id.as_ref()).collect();
        assert_eq!(ids, ["2", "3"]);
        assert!(
            matches!(
                &page.entries[0].accessible_by,
                Some(CollaborationGrantee::Group(group)) if group.id == GroupId::new("789")
            ),
            "accessible_by: {:?}",
            page.entries[0].accessible_by
        );

        Ok(())
    }

    /// オフセットまたはコラボレーション数の上限が範囲外の場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::offset_above_max(MAX_COLLABORATIONS_OFFSET + 1, None)]
    #[case::zero_limit(0, Some(0))]
    #[case::limit_above_max(0, Some(MAX_ITEMS_LIMIT + 1))]
    #[tokio::test]
    async fn test_list_group_collaborations_rejects_without_request(
        #[case] offset: u64,
        #[case] limit: Option<u32>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .list_group_collaborations(
                &GroupId::new("789"),
                offset,
                &ListGroupCollaborationsOptions { limit },
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// グループが存在しない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_list_group_collaborations_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .respond_with(not_found_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let result = client
            .list_group_collaborations(
                &GroupId::new("789"),
                0,
                &ListGroupCollaborationsOptions::default(),
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }

    /// `total_count`に達するまでページを送り、すべてのコラボレーションを順に返すことを確認
    ///
    /// `total_count`が上限の倍数でない場合、および倍数の場合のページ数も確認する。
    #[rstest]
    #[case::partial_last_page(5, 2, 3)]
    #[case::full_last_page(4, 2, 2)]
    #[case::no_collaborations(0, 2, 1)]
    #[tokio::test]
    async fn test_list_all_group_collaborations(
        #[case] total: u64,
        #[case] limit: u32,
        #[case] expected_pages: usize,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .respond_with(move |request: &Request| page_response(request, total))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let collaborations = client
            .list_all_group_collaborations(
                &GroupId::new("789"),
                &ListGroupCollaborationsOptions::default().with_limit(limit),
            )
            .await?;

        let ids: Vec<String> = collaborations.iter().map(|c| c.id.to_string()).collect();
        let expected: Vec<String> = (0..total).map(|i| i.to_string()).collect();
        assert_eq!(ids, expected);
        let pages = count_requests_to_path(&server, COLLABORATIONS_PATH).await;
        assert_eq!(pages, expected_pages);

        Ok(())
    }

    /// コラボレーション数の上限を省略した場合は、上限の1000件ずつ取得することを確認
    #[tokio::test]
    async fn test_list_all_group_collaborations_default_limit() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .and(query_param("offset", "0"))
            .and(query_param("limit", "1000"))
            .respond_with(|request: &Request| page_response(request, 1))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let collaborations = client
            .list_all_group_collaborations(
                &GroupId::new("789"),
                &ListGroupCollaborationsOptions::default(),
            )
            .await?;

        assert_eq!(collaborations.len(), 1);

        Ok(())
    }

    /// 次のページのオフセットが最大値を超える場合は、一部のコラボレーションだけを返さずにエラーを返すことを確認
    #[tokio::test]
    async fn test_list_all_group_collaborations_exceeds_max_offset() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .respond_with(|request: &Request| page_response(request, 20_000))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let result = client
            .list_all_group_collaborations(
                &GroupId::new("789"),
                &ListGroupCollaborationsOptions::default(),
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
mod test_list_pending_collaborations {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, Request, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    use super::*;
    use crate::test_support::{
        count_requests_to_path, create_client, mount_token, request_count, request_queries,
        short_retry,
    };

    /// コラボレーションのAPIのパス
    const COLLABORATIONS_PATH: &str = "/2.0/collaborations";

    /// 保留中のコラボレーションのJSONを返す。
    fn pending_json(id: u64) -> serde_json::Value {
        json!({
            "type": "collaboration",
            "id": id.to_string(),
            "item": null,
            "accessible_by": {"type": "user", "id": "1", "name": "", "login": "me@example.com"},
            "role": "editor",
            "status": "pending",
        })
    }

    /// `offset`から`limit`件の招待を、全体が`total`件として返すレスポンスを作成する。
    fn page_response(request: &Request, total: u64) -> ResponseTemplate {
        let param = |name: &str| {
            request
                .url
                .query_pairs()
                .find(|(key, _)| key == name)
                .and_then(|(_, value)| value.parse::<u64>().ok())
        };
        let offset = param("offset").unwrap_or(0);
        let limit = param("limit").unwrap_or(1000);
        let end = total.min(offset + limit);
        let entries: Vec<_> = (offset..end).map(pending_json).collect();
        ResponseTemplate::new(200).set_body_json(json!({
            "total_count": total,
            "limit": limit,
            "offset": offset,
            "entries": entries,
        }))
    }

    /// `status=pending`、`offset`および`limit`をクエリパラメーターで送信し、招待の1ページをデシリアライズすることを確認
    #[rstest]
    #[case::default(
        0,
        ListPendingCollaborationsOptions::default(),
        "status=pending&offset=0"
    )]
    #[case::with_limit(
        2,
        ListPendingCollaborationsOptions::default().with_limit(2),
        "status=pending&offset=2&limit=2"
    )]
    #[tokio::test]
    async fn test_list_pending_collaborations(
        #[case] offset: u64,
        #[case] options: ListPendingCollaborationsOptions,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .and(query_param("status", "pending"))
            .respond_with(|request: &Request| page_response(request, 5))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client.list_pending_collaborations(offset, &options).await?;

        assert_eq!(page.total_count, 5);
        assert_eq!(page.offset, offset);
        let first = page
            .entries
            .first()
            .ok_or_else(|| anyhow::anyhow!("no entries"))?;
        assert_eq!(first.id, CollaborationId::new(offset.to_string()));
        assert_eq!(first.status, Some(CollaborationStatus::Pending));
        assert_eq!(first.item, None);
        assert_eq!(
            request_queries(&server, COLLABORATIONS_PATH).await,
            vec![expected_query]
        );

        Ok(())
    }

    /// オフセットまたはコラボレーション数の上限が範囲外の場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::offset_above_max(MAX_COLLABORATIONS_OFFSET + 1, None)]
    #[case::zero_limit(0, Some(0))]
    #[case::limit_above_max(0, Some(MAX_ITEMS_LIMIT + 1))]
    #[tokio::test]
    async fn test_list_pending_collaborations_rejects_without_request(
        #[case] offset: u64,
        #[case] limit: Option<u32>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .list_pending_collaborations(offset, &ListPendingCollaborationsOptions { limit })
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// `total_count`に達するまでページを送り、すべての招待を順に返すことを確認
    #[rstest]
    #[case::partial_last_page(5, 2, 3)]
    #[case::no_invitations(0, 2, 1)]
    #[tokio::test]
    async fn test_list_all_pending_collaborations(
        #[case] total: u64,
        #[case] limit: u32,
        #[case] expected_pages: usize,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .and(query_param("status", "pending"))
            .respond_with(move |request: &Request| page_response(request, total))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let collaborations = client
            .list_all_pending_collaborations(
                &ListPendingCollaborationsOptions::default().with_limit(limit),
            )
            .await?;

        let ids: Vec<String> = collaborations.iter().map(|c| c.id.to_string()).collect();
        let expected: Vec<String> = (0..total).map(|i| i.to_string()).collect();
        assert_eq!(ids, expected);
        let pages = count_requests_to_path(&server, COLLABORATIONS_PATH).await;
        assert_eq!(pages, expected_pages);

        Ok(())
    }

    /// 次のページのオフセットが最大値を超える場合は、一部の招待だけを返さずにエラーを返すことを確認
    #[tokio::test]
    async fn test_list_all_pending_collaborations_exceeds_max_offset() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(COLLABORATIONS_PATH))
            .respond_with(|request: &Request| page_response(request, 20_000))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let result = client
            .list_all_pending_collaborations(&ListPendingCollaborationsOptions::default())
            .await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );

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
