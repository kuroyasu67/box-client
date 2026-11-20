//! ユーザーに関するAPI
//!
//! BOXのユーザーには、管理対象ユーザー（Managed User）、アプリユーザー（App User）および外部ユーザーがある。
//! ユーザーの作成、更新および削除は、管理者の権限を持つユーザーまたはアプリケーションだけが実行できる。

use garde::Validate;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, UserId,
    api::{
        pagination::{
            MAX_ITEMS_LIMIT, MAX_OFFSET, MarkerPage, collect_all_pages, marker_page_request,
            offset_page_request, truncate_to_max_entries,
        },
        types::validated,
        updates::{Nullable, NullableBody},
    },
    request::ApiRequest,
    retry::Idempotency,
};

/// ユーザー名の最大文字数
///
/// BOXのAPI仕様（OpenAPI）は、ユーザーの作成および更新で、ユーザー名を50文字以下に制限している。
pub const MAX_USER_NAME_CHARS: usize = 50;

/// ユーザーの役職の最大文字数
///
/// BOXのAPI仕様（OpenAPI）は、ユーザーの作成および更新で、役職を100文字以下に制限している。
pub const MAX_USER_JOB_TITLE_CHARS: usize = 100;

/// ユーザーの電話番号の最大文字数
///
/// BOXのAPI仕様（OpenAPI）は、ユーザーの作成および更新で、電話番号を100文字以下に制限している。
pub const MAX_USER_PHONE_CHARS: usize = 100;

/// ユーザーの住所の最大文字数
///
/// BOXのAPI仕様（OpenAPI）は、ユーザーの作成および更新で、住所を255文字以下に制限している。
pub const MAX_USER_ADDRESS_CHARS: usize = 255;

/// ストレージの容量を無制限にするときに、ユーザーの作成および更新で指定する値
pub const UNLIMITED_SPACE_AMOUNT: i64 = -1;

/// BOXのユーザー
///
/// BOXのユーザーのAPIが既定で返す属性（標準の表現）を受け取る。
/// BOXのAPI仕様（OpenAPI）は、`id`と`type`だけを必須とし、ほかの属性を省略可能としている。
/// BOXが属性を省略したりnullを返したりしても取得を失敗させないように、`id`と`type`以外の属性を`Option`で受け取る。
/// また、BOXのドキュメントによると、エンタープライズのコンテンツのコラボレーターである外部ユーザーの情報を取得した場合、
/// BOXは取得を許可しない属性をnullで返す。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct User {
    /// ユーザーID
    pub id: UserId,

    /// タイプ
    #[serde(rename = "type")]
    pub user_type: String,

    /// 表示名
    pub name: Option<String>,

    /// ログインに使用するメールアドレス
    pub login: Option<String>,

    /// 作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,

    /// 最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,

    /// 言語
    ///
    /// BOXは、ISO 639-1を変更した形式の言語コード（例えば`ja`や`en`）を返す。
    pub language: Option<String>,

    /// タイムゾーン
    ///
    /// BOXは、IANAのタイムゾーン名（例えば`Asia/Tokyo`）を返す。
    pub timezone: Option<String>,

    /// 使用できるストレージの容量（バイト）
    pub space_amount: Option<i64>,

    /// 使用しているストレージの容量（バイト）
    pub space_used: Option<i64>,

    /// アップロードできるファイルの最大サイズ（バイト）
    pub max_upload_size: Option<i64>,

    /// アカウントの状態
    pub status: Option<UserStatus>,

    /// 役職
    pub job_title: Option<String>,

    /// 電話番号
    pub phone: Option<String>,

    /// 住所
    pub address: Option<String>,

    /// アバター画像のURL
    pub avatar_url: Option<String>,

    /// 通知を送信する代わりのメールアドレス
    ///
    /// 代わりのメールアドレスを設定していない場合は`None`となる。
    pub notification_email: Option<NotificationEmail>,
}

/// 最小限のユーザー情報
///
/// フォルダ情報の作成者などとして、ほかのリソースに含まれる。
/// BOXのAPI仕様（OpenAPI）は、`id`と`type`だけを必須とし、表示名とメールアドレスを省略可能としている。
/// 1人のユーザーの属性が欠けているだけで、そのユーザーを含むフォルダ情報などの取得全体を失敗させないように、
/// `id`と`type`以外の属性を`Option`で受け取る。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct UserMini {
    /// ユーザーID
    pub id: UserId,

    /// タイプ
    #[serde(rename = "type")]
    pub user_type: String,

    /// 表示名
    pub name: Option<String>,

    /// ログインに使用するメールアドレス
    pub login: Option<String>,
}

/// ユーザーのアカウントの状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UserStatus {
    /// 有効
    Active,

    /// 無効
    Inactive,

    /// 削除と編集ができない
    CannotDeleteEdit,

    /// 削除、編集およびアップロードができない
    CannotDeleteEditUpload,

    /// BOXクライアントが知らない状態
    ///
    /// BOXが状態を追加しても、ユーザー情報の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

impl UserStatus {
    /// リクエストボディに指定する文字列を返す。
    ///
    /// [`UserStatus::Unknown`]は、BOXに送信する値を持たないため、`None`を返す。
    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Active => Some("active"),
            Self::Inactive => Some("inactive"),
            Self::CannotDeleteEdit => Some("cannot_delete_edit"),
            Self::CannotDeleteEditUpload => Some("cannot_delete_edit_upload"),
            Self::Unknown => None,
        }
    }
}

/// 通知を送信する代わりのメールアドレス
///
/// BOXは、代わりのメールアドレスを確認した後、主メールアドレスの代わりにこのメールアドレスへ通知を送信する。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct NotificationEmail {
    /// メールアドレス
    pub email: Option<String>,

    /// メールアドレスを確認済みかどうか
    pub is_confirmed: Option<bool>,
}

/// ユーザーの作成および更新で指定するエンタープライズの役割
///
/// BOXは、ユーザーの作成および更新で、管理者の役割を受け付けない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UserRole {
    /// 共同管理者
    Coadmin,

    /// ユーザー
    User,
}

impl UserRole {
    /// リクエストボディに指定する文字列を返す。
    fn as_str(self) -> &'static str {
        match self {
            Self::Coadmin => "coadmin",
            Self::User => "user",
        }
    }
}

/// トラッキングコード
///
/// 管理者は、トラッキングコードでユーザーに属性を割り当て、管理コンソールでレポートを作成できる。
/// BOXのドキュメントによると、エンタープライズでトラッキングコードを有効にしている場合だけ使用できる。
///
/// # Examples
///
/// ```
/// use box_client::TrackingCode;
///
/// let code = TrackingCode::new("department", "Sales");
/// assert_eq!(code.name, "department");
/// assert_eq!(code.value, "Sales");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrackingCode<'a> {
    /// トラッキングコードの名前
    pub name: &'a str,

    /// トラッキングコードの値
    pub value: &'a str,
}

impl<'a> TrackingCode<'a> {
    /// 名前と値からトラッキングコードを作成する。
    #[must_use]
    pub fn new(name: &'a str, value: &'a str) -> Self {
        Self { name, value }
    }
}

/// トラッキングコードのリクエストボディ
#[derive(Debug, Serialize)]
struct TrackingCodeBody<'a> {
    /// タイプ
    #[serde(rename = "type")]
    code_type: &'static str,

    /// トラッキングコードの名前
    name: &'a str,

    /// トラッキングコードの値
    value: &'a str,
}

impl<'a> TrackingCodeBody<'a> {
    /// トラッキングコードからボディを作成する。
    fn new(code: &TrackingCode<'a>) -> Self {
        Self {
            code_type: "tracking_code",
            name: code.name,
            value: code.value,
        }
    }
}

/// 作成するユーザーの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NewUserAccount<'a> {
    /// 管理対象ユーザー
    ///
    /// ログインに使用するメールアドレスを指定する。
    Managed {
        /// ログインに使用するメールアドレス
        login: &'a str,
    },

    /// アプリユーザー
    ///
    /// アプリユーザーは、BOXのWebアプリにログインできず、APIだけでBOXを使用する。メールアドレスを指定しない。
    AppUser,
}

/// ユーザーの作成のオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// `None`の属性は送信せず、BOXの既定値を使用する。
///
/// # Examples
///
/// ```
/// use box_client::{CreateUserOptions, UserRole};
///
/// let options = CreateUserOptions::default()
///     .with_role(UserRole::User)
///     .with_language("ja")
///     .with_timezone("Asia/Tokyo");
/// assert_eq!(options.role, Some(UserRole::User));
/// assert_eq!(options.language, Some("ja"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CreateUserOptions<'a> {
    /// エンタープライズの役割
    pub role: Option<UserRole>,

    /// 言語
    ///
    /// ISO 639-1を変更した形式の言語コード（例えば`ja`や`en`）を指定する。
    pub language: Option<&'a str>,

    /// タイムゾーン
    ///
    /// IANAのタイムゾーン名（例えば`Asia/Tokyo`）を指定する。
    pub timezone: Option<&'a str>,

    /// Box Syncを使用できるかどうか
    pub is_sync_enabled: Option<bool>,

    /// 役職
    ///
    /// [`MAX_USER_JOB_TITLE_CHARS`]文字以下で指定する。
    pub job_title: Option<&'a str>,

    /// 電話番号
    ///
    /// [`MAX_USER_PHONE_CHARS`]文字以下で指定する。
    pub phone: Option<&'a str>,

    /// 住所
    ///
    /// [`MAX_USER_ADDRESS_CHARS`]文字以下で指定する。
    pub address: Option<&'a str>,

    /// 使用できるストレージの容量（バイト）
    ///
    /// 容量を無制限にする場合は、[`UNLIMITED_SPACE_AMOUNT`]を指定する。
    pub space_amount: Option<i64>,

    /// トラッキングコード
    pub tracking_codes: Option<&'a [TrackingCode<'a>]>,

    /// 連絡先リストで、エンタープライズのほかのユーザーを参照できるかどうか
    pub can_see_managed_users: Option<bool>,

    /// エンタープライズの外部のユーザーとのコラボレーションを制限するかどうか
    pub is_external_collab_restricted: Option<bool>,

    /// エンタープライズのデバイスの制限を免除するかどうか
    pub is_exempt_from_device_limits: Option<bool>,

    /// 2要素認証を免除するかどうか
    pub is_exempt_from_login_verification: Option<bool>,

    /// アカウントの状態
    ///
    /// [`UserStatus::Unknown`]は指定できない。
    pub status: Option<UserStatus>,

    /// アプリユーザーの外部識別子
    ///
    /// 外部のIDプロバイダーのユーザーIDとBOXのユーザーを関連付けるために使用する。
    /// [`BoxClient::list_enterprise_users`]で、この値に一致するアプリユーザーを検索できる。
    pub external_app_user_id: Option<&'a str>,
}

impl<'a> CreateUserOptions<'a> {
    /// エンタープライズの役割を設定する。
    #[must_use]
    pub fn with_role(mut self, role: UserRole) -> Self {
        self.role = Some(role);
        self
    }

    /// 言語を設定する。
    #[must_use]
    pub fn with_language(mut self, language: &'a str) -> Self {
        self.language = Some(language);
        self
    }

    /// タイムゾーンを設定する。
    #[must_use]
    pub fn with_timezone(mut self, timezone: &'a str) -> Self {
        self.timezone = Some(timezone);
        self
    }

    /// Box Syncを使用できるかどうかを設定する。
    #[must_use]
    pub fn with_is_sync_enabled(mut self, is_sync_enabled: bool) -> Self {
        self.is_sync_enabled = Some(is_sync_enabled);
        self
    }

    /// 役職を設定する。
    #[must_use]
    pub fn with_job_title(mut self, job_title: &'a str) -> Self {
        self.job_title = Some(job_title);
        self
    }

    /// 電話番号を設定する。
    #[must_use]
    pub fn with_phone(mut self, phone: &'a str) -> Self {
        self.phone = Some(phone);
        self
    }

    /// 住所を設定する。
    #[must_use]
    pub fn with_address(mut self, address: &'a str) -> Self {
        self.address = Some(address);
        self
    }

    /// 使用できるストレージの容量（バイト）を設定する。
    #[must_use]
    pub fn with_space_amount(mut self, space_amount: i64) -> Self {
        self.space_amount = Some(space_amount);
        self
    }

    /// トラッキングコードを設定する。
    #[must_use]
    pub fn with_tracking_codes(mut self, tracking_codes: &'a [TrackingCode<'a>]) -> Self {
        self.tracking_codes = Some(tracking_codes);
        self
    }

    /// 連絡先リストで、エンタープライズのほかのユーザーを参照できるかどうかを設定する。
    #[must_use]
    pub fn with_can_see_managed_users(mut self, can_see_managed_users: bool) -> Self {
        self.can_see_managed_users = Some(can_see_managed_users);
        self
    }

    /// エンタープライズの外部のユーザーとのコラボレーションを制限するかどうかを設定する。
    #[must_use]
    pub fn with_is_external_collab_restricted(
        mut self,
        is_external_collab_restricted: bool,
    ) -> Self {
        self.is_external_collab_restricted = Some(is_external_collab_restricted);
        self
    }

    /// エンタープライズのデバイスの制限を免除するかどうかを設定する。
    #[must_use]
    pub fn with_is_exempt_from_device_limits(mut self, is_exempt_from_device_limits: bool) -> Self {
        self.is_exempt_from_device_limits = Some(is_exempt_from_device_limits);
        self
    }

    /// 2要素認証を免除するかどうかを設定する。
    #[must_use]
    pub fn with_is_exempt_from_login_verification(
        mut self,
        is_exempt_from_login_verification: bool,
    ) -> Self {
        self.is_exempt_from_login_verification = Some(is_exempt_from_login_verification);
        self
    }

    /// アカウントの状態を設定する。
    #[must_use]
    pub fn with_status(mut self, status: UserStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// アプリユーザーの外部識別子を設定する。
    #[must_use]
    pub fn with_external_app_user_id(mut self, external_app_user_id: &'a str) -> Self {
        self.external_app_user_id = Some(external_app_user_id);
        self
    }
}

/// ユーザーの作成と更新で共通する属性のリクエストボディ
///
/// `None`の属性は送信しない。`garde`は`None`の場合に`length`と`range`を検証しない。
#[derive(Debug, Serialize, Validate)]
struct UserAttributesBody<'a> {
    /// エンタープライズの役割
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    role: Option<&'static str>,

    /// 言語
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    language: Option<&'a str>,

    /// タイムゾーン
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    timezone: Option<&'a str>,

    /// Box Syncを使用できるかどうか
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    is_sync_enabled: Option<bool>,

    /// 役職
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(length(chars, max = MAX_USER_JOB_TITLE_CHARS))]
    job_title: Option<&'a str>,

    /// 電話番号
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(length(chars, max = MAX_USER_PHONE_CHARS))]
    phone: Option<&'a str>,

    /// 住所
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(length(chars, max = MAX_USER_ADDRESS_CHARS))]
    address: Option<&'a str>,

    /// 使用できるストレージの容量（バイト）
    ///
    /// BOXは、無制限を表す`-1`より小さい値を受け付けない。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(range(min = UNLIMITED_SPACE_AMOUNT))]
    space_amount: Option<i64>,

    /// トラッキングコード
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    tracking_codes: Option<Vec<TrackingCodeBody<'a>>>,

    /// 連絡先リストで、エンタープライズのほかのユーザーを参照できるかどうか
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    can_see_managed_users: Option<bool>,

    /// エンタープライズの外部のユーザーとのコラボレーションを制限するかどうか
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    is_external_collab_restricted: Option<bool>,

    /// エンタープライズのデバイスの制限を免除するかどうか
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    is_exempt_from_device_limits: Option<bool>,

    /// 2要素認証を免除するかどうか
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    is_exempt_from_login_verification: Option<bool>,

    /// アカウントの状態
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    status: Option<&'static str>,

    /// アプリユーザーの外部識別子
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    external_app_user_id: Option<&'a str>,
}

/// ユーザーのアカウントの状態をリクエストボディの値に変換する。
///
/// # Errors
///
/// `status`が[`UserStatus::Unknown`]の場合は、[`BoxClientError::InvalidArgument`]を返す。
fn status_str(status: Option<UserStatus>) -> BoxClientResult<Option<&'static str>> {
    status
        .map(|status| {
            status.as_str().ok_or(BoxClientError::InvalidArgument(
                "status of a user must not be unknown",
            ))
        })
        .transpose()
}

/// メールアドレスが空文字列または空白だけでないことを検証する。
///
/// # Errors
///
/// メールアドレスが空文字列または空白だけの場合は、`message`の[`BoxClientError::InvalidArgument`]を返す。
fn ensure_email_not_empty(email: &str, message: &'static str) -> BoxClientResult<()> {
    if email.trim().is_empty() {
        return Err(BoxClientError::InvalidArgument(message));
    }
    Ok(())
}

/// ユーザーの作成リクエストのボディ
#[derive(Debug, Serialize, Validate)]
struct CreateUserBody<'a> {
    /// ユーザー名
    #[garde(length(chars, min = 1, max = MAX_USER_NAME_CHARS))]
    name: &'a str,

    /// ログインに使用するメールアドレス
    ///
    /// アプリユーザーを作成する場合は送信しない。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    login: Option<&'a str>,

    /// アプリユーザーかどうか
    ///
    /// 管理対象ユーザーを作成する場合は送信しない。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    is_platform_access_only: Option<bool>,

    /// 作成と更新で共通する属性
    #[serde(flatten)]
    #[garde(dive)]
    attributes: UserAttributesBody<'a>,
}

impl<'a> CreateUserBody<'a> {
    /// 引数とオプションからボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// 管理対象ユーザーのメールアドレスが空文字列もしくは空白だけの場合、
    /// または`options.status`が[`UserStatus::Unknown`]の場合は、[`BoxClientError::InvalidArgument`]を返す。
    /// ユーザー名、役職、電話番号、住所またはストレージの容量をBOXが拒否する場合は、
    /// [`BoxClientError::InvalidRequestBody`]を返す。
    fn new(
        name: &'a str,
        account: NewUserAccount<'a>,
        options: &CreateUserOptions<'a>,
    ) -> BoxClientResult<Self> {
        let (login, is_platform_access_only) = match account {
            NewUserAccount::Managed { login } => {
                ensure_email_not_empty(login, "login of a managed user must not be empty")?;
                (Some(login), None)
            }
            NewUserAccount::AppUser => (None, Some(true)),
        };

        validated(Self {
            name,
            login,
            is_platform_access_only,
            attributes: UserAttributesBody {
                role: options.role.map(UserRole::as_str),
                language: options.language,
                timezone: options.timezone,
                is_sync_enabled: options.is_sync_enabled,
                job_title: options.job_title,
                phone: options.phone,
                address: options.address,
                space_amount: options.space_amount,
                tracking_codes: options
                    .tracking_codes
                    .map(|codes| codes.iter().map(TrackingCodeBody::new).collect()),
                can_see_managed_users: options.can_see_managed_users,
                is_external_collab_restricted: options.is_external_collab_restricted,
                is_exempt_from_device_limits: options.is_exempt_from_device_limits,
                is_exempt_from_login_verification: options.is_exempt_from_login_verification,
                status: status_str(options.status)?,
                external_app_user_id: options.external_app_user_id,
            },
        })
    }
}

/// ユーザーの更新で変更する属性
///
/// `None`の属性は送信せず、BOXは変更しない。
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// # Examples
///
/// ```
/// use box_client::{Nullable, UserStatus, UserUpdate};
///
/// // 役職を変更して、アカウントを無効にする。
/// let update = UserUpdate::default()
///     .with_job_title("Manager")
///     .with_status(UserStatus::Inactive);
/// assert_eq!(update.job_title, Some("Manager"));
///
/// // 通知を送信する代わりのメールアドレスを削除する。
/// let update = UserUpdate::default().with_notification_email_removed();
/// assert_eq!(update.notification_email, Some(Nullable::Null));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct UserUpdate<'a> {
    /// ユーザー名
    ///
    /// 1文字以上[`MAX_USER_NAME_CHARS`]文字以下で指定する。
    pub name: Option<&'a str>,

    /// ログインに使用するメールアドレス
    ///
    /// BOXのドキュメントによると、ユーザーがメールアドレスを確認していない場合は、主メールアドレスを変更できない。
    pub login: Option<&'a str>,

    /// エンタープライズの役割
    pub role: Option<UserRole>,

    /// 言語
    ///
    /// ISO 639-1を変更した形式の言語コード（例えば`ja`や`en`）を指定する。
    pub language: Option<&'a str>,

    /// タイムゾーン
    ///
    /// IANAのタイムゾーン名（例えば`Asia/Tokyo`）を指定する。
    pub timezone: Option<&'a str>,

    /// Box Syncを使用できるかどうか
    pub is_sync_enabled: Option<bool>,

    /// 役職
    ///
    /// [`MAX_USER_JOB_TITLE_CHARS`]文字以下で指定する。
    pub job_title: Option<&'a str>,

    /// 電話番号
    ///
    /// [`MAX_USER_PHONE_CHARS`]文字以下で指定する。
    pub phone: Option<&'a str>,

    /// 住所
    ///
    /// [`MAX_USER_ADDRESS_CHARS`]文字以下で指定する。
    pub address: Option<&'a str>,

    /// 使用できるストレージの容量（バイト）
    ///
    /// 容量を無制限にする場合は、[`UNLIMITED_SPACE_AMOUNT`]を指定する。
    pub space_amount: Option<i64>,

    /// トラッキングコード
    pub tracking_codes: Option<&'a [TrackingCode<'a>]>,

    /// 連絡先リストで、エンタープライズのほかのユーザーを参照できるかどうか
    pub can_see_managed_users: Option<bool>,

    /// エンタープライズの外部のユーザーとのコラボレーションを制限するかどうか
    pub is_external_collab_restricted: Option<bool>,

    /// エンタープライズのデバイスの制限を免除するかどうか
    pub is_exempt_from_device_limits: Option<bool>,

    /// 2要素認証を免除するかどうか
    pub is_exempt_from_login_verification: Option<bool>,

    /// 次のログインでパスワードの再設定を求めるかどうか
    pub is_password_reset_required: Option<bool>,

    /// ユーザーが無効なときに、コラボレーターがユーザーの所有するコンテンツにアクセスできるかどうか
    ///
    /// BOXのドキュメントによると、この設定は既存のコラボレーターのアクセスを維持するだけで、新しい権限を与えない。
    pub is_collaborated_content_available_when_owner_inactive: Option<bool>,

    /// アカウントの状態
    ///
    /// [`UserStatus::Unknown`]は指定できない。
    pub status: Option<UserStatus>,

    /// 通知を送信する代わりのメールアドレス
    ///
    /// [`Nullable::Value`]を指定するとメールアドレスを設定し、[`Nullable::Null`]を指定すると削除する。
    /// BOXのドキュメントによると、エンタープライズで代わりのメールアドレスの更新を禁止している場合、
    /// BOXはエラーコードが`access_denied_insufficient_permissions`の403を返す。
    pub notification_email: Option<Nullable<&'a str>>,

    /// アプリユーザーの外部識別子
    ///
    /// BOXのドキュメントによると、アプリユーザーを作成したアプリケーションで取得したアクセストークンでだけ更新できる。
    pub external_app_user_id: Option<&'a str>,

    /// ユーザーをエンタープライズから除外して、無料のユーザーにするかどうか
    ///
    /// `true`の場合は、エンタープライズにnullを送信する。元に戻せない操作であるため、注意して指定する。
    /// BOXのドキュメントによると、ユーザーがコンテンツを所有し、エンタープライズにリテンションポリシーまたはリーガルホールドがある場合、
    /// BOXはエラーコードが`user_must_not_have_retained_content`の400を返す。
    pub roll_out_of_enterprise: bool,

    /// エンタープライズから除外したことを、ユーザーにメールで通知するかどうか
    ///
    /// [`UserUpdate::roll_out_of_enterprise`]が`true`の場合だけ指定できる。
    pub notify: Option<bool>,
}

impl<'a> UserUpdate<'a> {
    /// ユーザー名を設定する。
    #[must_use]
    pub fn with_name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    /// ログインに使用するメールアドレスを設定する。
    #[must_use]
    pub fn with_login(mut self, login: &'a str) -> Self {
        self.login = Some(login);
        self
    }

    /// エンタープライズの役割を設定する。
    #[must_use]
    pub fn with_role(mut self, role: UserRole) -> Self {
        self.role = Some(role);
        self
    }

    /// 言語を設定する。
    #[must_use]
    pub fn with_language(mut self, language: &'a str) -> Self {
        self.language = Some(language);
        self
    }

    /// タイムゾーンを設定する。
    #[must_use]
    pub fn with_timezone(mut self, timezone: &'a str) -> Self {
        self.timezone = Some(timezone);
        self
    }

    /// Box Syncを使用できるかどうかを設定する。
    #[must_use]
    pub fn with_is_sync_enabled(mut self, is_sync_enabled: bool) -> Self {
        self.is_sync_enabled = Some(is_sync_enabled);
        self
    }

    /// 役職を設定する。
    #[must_use]
    pub fn with_job_title(mut self, job_title: &'a str) -> Self {
        self.job_title = Some(job_title);
        self
    }

    /// 電話番号を設定する。
    #[must_use]
    pub fn with_phone(mut self, phone: &'a str) -> Self {
        self.phone = Some(phone);
        self
    }

    /// 住所を設定する。
    #[must_use]
    pub fn with_address(mut self, address: &'a str) -> Self {
        self.address = Some(address);
        self
    }

    /// 使用できるストレージの容量（バイト）を設定する。
    #[must_use]
    pub fn with_space_amount(mut self, space_amount: i64) -> Self {
        self.space_amount = Some(space_amount);
        self
    }

    /// トラッキングコードを設定する。
    #[must_use]
    pub fn with_tracking_codes(mut self, tracking_codes: &'a [TrackingCode<'a>]) -> Self {
        self.tracking_codes = Some(tracking_codes);
        self
    }

    /// 連絡先リストで、エンタープライズのほかのユーザーを参照できるかどうかを設定する。
    #[must_use]
    pub fn with_can_see_managed_users(mut self, can_see_managed_users: bool) -> Self {
        self.can_see_managed_users = Some(can_see_managed_users);
        self
    }

    /// エンタープライズの外部のユーザーとのコラボレーションを制限するかどうかを設定する。
    #[must_use]
    pub fn with_is_external_collab_restricted(
        mut self,
        is_external_collab_restricted: bool,
    ) -> Self {
        self.is_external_collab_restricted = Some(is_external_collab_restricted);
        self
    }

    /// エンタープライズのデバイスの制限を免除するかどうかを設定する。
    #[must_use]
    pub fn with_is_exempt_from_device_limits(mut self, is_exempt_from_device_limits: bool) -> Self {
        self.is_exempt_from_device_limits = Some(is_exempt_from_device_limits);
        self
    }

    /// 2要素認証を免除するかどうかを設定する。
    #[must_use]
    pub fn with_is_exempt_from_login_verification(
        mut self,
        is_exempt_from_login_verification: bool,
    ) -> Self {
        self.is_exempt_from_login_verification = Some(is_exempt_from_login_verification);
        self
    }

    /// 次のログインでパスワードの再設定を求めるかどうかを設定する。
    #[must_use]
    pub fn with_is_password_reset_required(mut self, is_password_reset_required: bool) -> Self {
        self.is_password_reset_required = Some(is_password_reset_required);
        self
    }

    /// ユーザーが無効なときに、コラボレーターがユーザーの所有するコンテンツにアクセスできるかどうかを設定する。
    #[must_use]
    pub fn with_is_collaborated_content_available_when_owner_inactive(
        mut self,
        available: bool,
    ) -> Self {
        self.is_collaborated_content_available_when_owner_inactive = Some(available);
        self
    }

    /// アカウントの状態を設定する。
    #[must_use]
    pub fn with_status(mut self, status: UserStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// 通知を送信する代わりのメールアドレスを設定する。
    #[must_use]
    pub fn with_notification_email(mut self, email: &'a str) -> Self {
        self.notification_email = Some(Nullable::Value(email));
        self
    }

    /// 通知を送信する代わりのメールアドレスを削除する。
    #[must_use]
    pub fn with_notification_email_removed(mut self) -> Self {
        self.notification_email = Some(Nullable::Null);
        self
    }

    /// アプリユーザーの外部識別子を設定する。
    #[must_use]
    pub fn with_external_app_user_id(mut self, external_app_user_id: &'a str) -> Self {
        self.external_app_user_id = Some(external_app_user_id);
        self
    }

    /// ユーザーをエンタープライズから除外して、無料のユーザーにする。
    #[must_use]
    pub fn with_roll_out_of_enterprise(mut self) -> Self {
        self.roll_out_of_enterprise = true;
        self
    }

    /// エンタープライズから除外したことを、ユーザーにメールで通知するかどうかを設定する。
    #[must_use]
    pub fn with_notify(mut self, notify: bool) -> Self {
        self.notify = Some(notify);
        self
    }
}

/// 通知を送信する代わりのメールアドレスのリクエストボディ
#[derive(Debug, Serialize)]
struct NotificationEmailBody<'a> {
    /// メールアドレス
    email: &'a str,
}

/// ユーザーの更新リクエストのボディ
#[derive(Debug, Serialize, Validate)]
struct UpdateUserBody<'a> {
    /// ユーザー名
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(length(chars, min = 1, max = MAX_USER_NAME_CHARS))]
    name: Option<&'a str>,

    /// ログインに使用するメールアドレス
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    login: Option<&'a str>,

    /// 作成と更新で共通する属性
    #[serde(flatten)]
    #[garde(dive)]
    attributes: UserAttributesBody<'a>,

    /// 次のログインでパスワードの再設定を求めるかどうか
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    is_password_reset_required: Option<bool>,

    /// ユーザーが無効なときに、コラボレーターがユーザーの所有するコンテンツにアクセスできるかどうか
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    is_collaborated_content_available_when_owner_inactive: Option<bool>,

    /// 通知を送信する代わりのメールアドレス
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    notification_email: Option<NullableBody<NotificationEmailBody<'a>>>,

    /// エンタープライズ
    ///
    /// ユーザーをエンタープライズから除外する場合だけ、nullを送信する。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    enterprise: Option<NullableBody<()>>,

    /// エンタープライズから除外したことを、ユーザーにメールで通知するかどうか
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    notify: Option<bool>,
}

impl<'a> UpdateUserBody<'a> {
    /// 変更する属性からボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// メールアドレスもしくは通知を送信する代わりのメールアドレスが空文字列もしくは空白だけの場合、
    /// `update.status`が[`UserStatus::Unknown`]の場合、
    /// または`update.roll_out_of_enterprise`が`false`で`update.notify`を指定した場合は、
    /// [`BoxClientError::InvalidArgument`]を返す。
    /// ユーザー名、役職、電話番号、住所またはストレージの容量をBOXが拒否する場合は、
    /// [`BoxClientError::InvalidRequestBody`]を返す。
    fn new(update: &UserUpdate<'a>) -> BoxClientResult<Self> {
        if let Some(login) = update.login {
            ensure_email_not_empty(login, "login of a user must not be empty")?;
        }
        if let Some(Nullable::Value(email)) = update.notification_email {
            ensure_email_not_empty(email, "notification email of a user must not be empty")?;
        }
        if !update.roll_out_of_enterprise && update.notify.is_some() {
            return Err(BoxClientError::InvalidArgument(
                "notify can only be used when rolling the user out of the enterprise",
            ));
        }

        validated(Self {
            name: update.name,
            login: update.login,
            attributes: UserAttributesBody {
                role: update.role.map(UserRole::as_str),
                language: update.language,
                timezone: update.timezone,
                is_sync_enabled: update.is_sync_enabled,
                job_title: update.job_title,
                phone: update.phone,
                address: update.address,
                space_amount: update.space_amount,
                tracking_codes: update
                    .tracking_codes
                    .map(|codes| codes.iter().map(TrackingCodeBody::new).collect()),
                can_see_managed_users: update.can_see_managed_users,
                is_external_collab_restricted: update.is_external_collab_restricted,
                is_exempt_from_device_limits: update.is_exempt_from_device_limits,
                is_exempt_from_login_verification: update.is_exempt_from_login_verification,
                status: status_str(update.status)?,
                external_app_user_id: update.external_app_user_id,
            },
            is_password_reset_required: update.is_password_reset_required,
            is_collaborated_content_available_when_owner_inactive: update
                .is_collaborated_content_available_when_owner_inactive,
            notification_email: update
                .notification_email
                .map(|email| email.to_body(|&email| NotificationEmailBody { email })),
            enterprise: update.roll_out_of_enterprise.then_some(NullableBody::Null),
            notify: update.notify,
        })
    }
}

/// ユーザーの削除のオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// # Examples
///
/// ```
/// use box_client::DeleteUserOptions;
///
/// let options = DeleteUserOptions::default().with_notify(false).with_force(true);
/// assert_eq!(options.notify, Some(false));
/// assert_eq!(options.force, Some(true));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeleteUserOptions {
    /// 削除したことを、ユーザーにメールで通知するかどうか
    ///
    /// `None`の場合はBOXの既定値を使用する。
    pub notify: Option<bool>,

    /// ユーザーがファイルを所有していても削除するかどうか
    ///
    /// `true`の場合は、ユーザーが所有するファイルも削除する。`None`の場合はBOXの既定値を使用し、
    /// ユーザーがコンテンツを所有していると削除に失敗する。
    /// BOXのドキュメントによると、エンタープライズにリテンションポリシー（廃止したものを含む）または
    /// 解除していないリーガルホールドがある場合、`true`を指定してもコンテンツを所有するユーザーを削除できない。
    pub force: Option<bool>,
}

impl DeleteUserOptions {
    /// 削除したことを、ユーザーにメールで通知するかどうかを設定する。
    #[must_use]
    pub fn with_notify(mut self, notify: bool) -> Self {
        self.notify = Some(notify);
        self
    }

    /// ユーザーがファイルを所有していても削除するかどうかを設定する。
    #[must_use]
    pub fn with_force(mut self, force: bool) -> Self {
        self.force = Some(force);
        self
    }
}

/// エンタープライズのユーザーをリストするときに、取得するユーザーの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UserTypeFilter {
    /// すべての種類のユーザー
    ///
    /// BOXのドキュメントによると、外部ユーザーは、メールアドレスが検索語と完全に一致する場合だけ返す。
    All,

    /// 管理対象ユーザーとアプリユーザー
    Managed,

    /// 外部ユーザー
    ///
    /// BOXのドキュメントによると、メールアドレスが検索語と完全に一致する外部ユーザーだけを返す。
    External,
}

impl UserTypeFilter {
    /// クエリパラメーターに指定する文字列を返す。
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Managed => "managed",
            Self::External => "external",
        }
    }
}

/// エンタープライズのユーザーをリストするときのオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// 検索語（[`ListEnterpriseUsersOptions::filter_term`]）またはアプリユーザーの外部識別子
/// （[`ListEnterpriseUsersOptions::external_app_user_id`]）を指定した場合、BOXクライアントはオフセット方式でページを送り、
/// 条件に一致するユーザーのうち、先頭から[`MAX_FILTERED_USERS`]件までしか返さない。
/// したがって、条件に一致するユーザーが[`MAX_FILTERED_USERS`]件より少なくなるように、検索語または外部識別子を指定する。
/// 理由は[`BoxClient::list_enterprise_users`]を参照する。
///
/// # Examples
///
/// ```
/// use box_client::{ListEnterpriseUsersOptions, UserTypeFilter};
///
/// let options = ListEnterpriseUsersOptions::default()
///     .with_filter_term("john")
///     .with_user_type(UserTypeFilter::Managed)
///     .with_limit(100);
/// assert_eq!(options.filter_term, Some("john"));
/// assert_eq!(options.limit, Some(100));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListEnterpriseUsersOptions<'a> {
    /// 検索語
    ///
    /// 名前またはメールアドレスが検索語で始まるユーザーだけを返す。
    /// BOXのドキュメントによると、外部ユーザーは、メールアドレスが検索語と完全に一致する場合だけ返す。
    pub filter_term: Option<&'a str>,

    /// 取得するユーザーの種類
    ///
    /// `None`の場合はBOXの既定値を使用する。
    pub user_type: Option<UserTypeFilter>,

    /// アプリユーザーの外部識別子
    ///
    /// 外部識別子がこの値と一致するアプリユーザーだけを返す。
    pub external_app_user_id: Option<&'a str>,

    /// 1ページで取得するユーザー数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,
}

impl<'a> ListEnterpriseUsersOptions<'a> {
    /// 検索語を設定する。
    #[must_use]
    pub fn with_filter_term(mut self, filter_term: &'a str) -> Self {
        self.filter_term = Some(filter_term);
        self
    }

    /// 取得するユーザーの種類を設定する。
    #[must_use]
    pub fn with_user_type(mut self, user_type: UserTypeFilter) -> Self {
        self.user_type = Some(user_type);
        self
    }

    /// アプリユーザーの外部識別子を設定する。
    #[must_use]
    pub fn with_external_app_user_id(mut self, external_app_user_id: &'a str) -> Self {
        self.external_app_user_id = Some(external_app_user_id);
        self
    }

    /// 1ページで取得するユーザー数の上限を設定する。
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。範囲外の値は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// 検索語またはアプリユーザーの外部識別子でユーザーを絞り込むかどうかを返す。
    ///
    /// 絞り込む場合、BOXクライアントはオフセット方式でページを送る。
    fn is_filtered(&self) -> bool {
        self.filter_term.is_some() || self.external_app_user_id.is_some()
    }
}

/// 検索語またはアプリユーザーの外部識別子を指定してエンタープライズのユーザーをリストするときに、取得できるユーザー数の上限
///
/// BOXクライアントは、検索語またはアプリユーザーの外部識別子を指定した場合、オフセット方式でページを送る。
/// BOXはオフセットが10,000を超えるリクエストを400で拒否するため、BOXクライアントは、条件に一致するユーザーのうち、
/// 先頭からこの件数までしか返さない。
pub const MAX_FILTERED_USERS: u64 = MAX_OFFSET;

/// オフセット方式でページを送るときに、マーカーからオフセットを取り出して検証する。
///
/// オフセット方式では、BOXクライアントが次のページのオフセットを10進数の文字列にして、[`UsersPage::next_marker`]に設定する。
/// BOXクライアントは、[`MAX_FILTERED_USERS`]以上のオフセットをマーカーに設定しない。
/// 最初のページを取得する場合（`marker`が`None`）は、オフセットを0とする。
///
/// # Errors
///
/// マーカーが、BOXクライアントがオフセット方式で設定する値（[`MAX_FILTERED_USERS`]未満の10進数の整数）でない場合は、
/// [`BoxClientError::InvalidArgument`]を返す。
fn users_offset(marker: Option<&str>) -> BoxClientResult<u64> {
    let Some(marker) = marker else {
        return Ok(0);
    };
    marker
        .parse::<u64>()
        .ok()
        .filter(|&offset| offset < MAX_FILTERED_USERS)
        .ok_or(BoxClientError::InvalidArgument(
            "marker must be the next_marker returned for the same filter_term or external_app_user_id",
        ))
}

/// エンタープライズのユーザーの1ページ
///
/// BOXクライアントは、検索語およびアプリユーザーの外部識別子を指定しない場合はマーカー方式で、
/// 指定した場合はオフセット方式でページを送る。どちらの場合も、[`UsersPage::next_marker`]を次の呼び出しに渡すと、
/// 次のページを取得できる。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct UsersPage {
    /// このページのユーザー
    pub entries: Vec<User>,

    /// このページの取得に使用したユーザー数の上限
    ///
    /// BOXがレスポンスに含めない場合は`None`になる。
    pub limit: Option<u32>,

    /// 次のページを取得するためのマーカー
    ///
    /// 後続のユーザーがない場合は`None`になる。
    /// オフセット方式の場合は、BOXクライアントが次のページのオフセットを10進数の文字列にして設定する。
    /// このマーカーは、同じオプションで呼び出す場合だけ使用できる。
    pub next_marker: Option<String>,
}

/// オフセット方式で取得したエンタープライズのユーザーの1ページのレスポンス
#[derive(Debug, Deserialize)]
struct UsersOffsetPage {
    /// このページのユーザー
    entries: Vec<User>,

    /// 最後のユーザーのオフセットに1を加えた値
    total_count: u64,

    /// このページの取得に使用したユーザー数の上限
    limit: Option<u32>,
}

impl UsersOffsetPage {
    /// オフセット方式のページを、次のページのオフセットをマーカーにした[`UsersPage`]に変換する。
    ///
    /// BOXが返す`offset`ではなく、要求した`offset`から次のページの位置を求める。
    /// BOXが空のページを返した場合、または次のページの位置が`total_count`に達した場合は、後続のユーザーがないとする。
    ///
    /// 先頭から[`MAX_FILTERED_USERS`]件までしか返さないように、上限を超えるユーザーをページから取り除き、
    /// 次のページの位置が上限に達した場合も、後続のユーザーがないとする。
    fn into_users_page(self, offset: u64) -> UsersPage {
        let mut entries = self.entries;
        truncate_to_max_entries(&mut entries, offset, MAX_FILTERED_USERS);
        let next_offset = offset + entries.len() as u64;
        let next_marker = (!entries.is_empty()
            && next_offset < self.total_count
            && next_offset < MAX_FILTERED_USERS)
            .then(|| next_offset.to_string());
        UsersPage {
            entries,
            limit: self.limit,
            next_marker,
        }
    }
}

impl MarkerPage for UsersPage {
    type Entry = User;

    fn into_parts(self) -> (Vec<User>, Option<String>) {
        (self.entries, self.next_marker)
    }
}

impl BoxClient {
    /// 認証中のユーザーを取得する。
    ///
    /// JWT認証でエンタープライズとして認証している場合は、アプリケーションのサービスアカウントを返す。
    ///
    /// # Errors
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn me(&self) -> BoxClientResult<User> {
        self.send_json(ApiRequest::get(["users", "me"])).await
    }

    /// ユーザー情報を取得する。
    ///
    /// BOXのドキュメントによると、アプリケーションと認証中のユーザーが、エンタープライズのすべてのユーザーを
    /// 参照する権限を持つ必要がある。エンタープライズのコンテンツのコラボレーターである外部ユーザーの情報も取得できるが、
    /// BOXは取得を許可しない属性をnullで返す。
    ///
    /// # Arguments
    ///
    /// - `user_id`: 情報を取得するユーザーのID
    ///
    /// # Errors
    ///
    /// ユーザーが存在しない場合、またはユーザーを参照する権限がない場合は、
    /// [`BoxClientError::Api`]を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn user_information(&self, user_id: &UserId) -> BoxClientResult<User> {
        self.send_json(ApiRequest::get(["users", user_id.as_ref()]))
            .await
    }

    /// エンタープライズのユーザーを1ページ分リストする。
    ///
    /// BOXのドキュメントによると、アプリケーションと認証中のユーザーが、エンタープライズのすべてのユーザーを
    /// 参照する権限を持つ必要がある。
    ///
    /// BOXはマーカー方式とオフセット方式のページ送りに対応する。BOXクライアントは、オプションによって次のように方式を切り替える。
    ///
    /// - 検索語およびアプリユーザーの外部識別子を指定しない場合は、マーカー方式でページを送る。
    ///   オフセット方式では、BOXがオフセットが10,000を超えるリクエストを拒否し、
    ///   それより後ろのユーザーを取得できないためである。
    /// - 検索語またはアプリユーザーの外部識別子を指定した場合は、オフセット方式でページを送る。
    ///   BOXのドキュメントには記載がないが、2026年11月20日に確認したところ、マーカー方式でこれらを指定すると、
    ///   BOXはマーカーを無視して同じページと同じ`next_marker`を返し続け、次のページに進めなかった。
    ///   また、外部識別子を指定すると、BOXは`entries`の代わりに`users`でユーザーを返し、`next_marker`を返さなかった。
    ///   一方、オフセット方式では、BOXはどちらを指定してもドキュメントのとおりの形式で応答し、正しくページを送れた。
    ///
    /// そのため、検索語またはアプリユーザーの外部識別子を指定した場合は、条件に一致するユーザーのうち、
    /// 先頭から[`MAX_FILTERED_USERS`]件までしか返さない。上限に達すると、後続のユーザーが残っていても、
    /// [`UsersPage::next_marker`]を`None`にする。したがって、条件に一致するユーザーが[`MAX_FILTERED_USERS`]件より
    /// 少なくなるように、検索語または外部識別子を指定する。
    ///
    /// 1回の呼び出しで返すユーザーは最大で`limit`件であるため、後続のユーザーが残っている場合がある。
    /// 後続のユーザーがある場合は、どちらの方式でも、戻り値の[`UsersPage::next_marker`]が`Some`になる。
    /// そのマーカーを`marker`に指定し、同じオプションで再度呼び出すと、次のページを取得できる。
    /// すべてのユーザーが必要な場合は、[`BoxClient::list_all_enterprise_users`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `marker`: 前のページの[`UsersPage::next_marker`]。最初のページを取得する場合は`None`
    /// - `options`: 検索語、ユーザーの種類およびユーザー数の上限などのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// 検索語またはアプリユーザーの外部識別子を指定した場合に、`marker`が同じオプションで取得したマーカーでないときも、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// エンタープライズのユーザーを参照する権限がない場合は、[`BoxClientError::Api`]を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_enterprise_users(
        &self,
        marker: Option<&str>,
        options: &ListEnterpriseUsersOptions<'_>,
    ) -> BoxClientResult<UsersPage> {
        if !options.is_filtered() {
            let mut request = marker_page_request(["users"], marker, options.limit, None, None)?;
            if let Some(user_type) = options.user_type {
                request = request.query("user_type", user_type.as_str());
            }
            return self.send_json(request).await;
        }

        let offset = users_offset(marker)?;
        let mut request = offset_page_request(["users"], offset, options.limit)?;
        if let Some(filter_term) = options.filter_term {
            request = request.query("filter_term", filter_term);
        }
        if let Some(user_type) = options.user_type {
            request = request.query("user_type", user_type.as_str());
        }
        if let Some(external_app_user_id) = options.external_app_user_id {
            request = request.query("external_app_user_id", external_app_user_id);
        }

        let page: UsersOffsetPage = self.send_json(request).await?;
        Ok(page.into_users_page(offset))
    }

    /// エンタープライズのすべてのユーザーを、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。
    ///
    /// 検索語またはアプリユーザーの外部識別子を指定した場合は、[`BoxClient::list_enterprise_users`]と同じ理由で
    /// オフセット方式でページを送るため、条件に一致するユーザーのうち、先頭から[`MAX_FILTERED_USERS`]件までしか返さない。
    /// 条件に一致するユーザーがそれより多くても、エラーにはせず、先頭から[`MAX_FILTERED_USERS`]件を返す。
    /// したがって、条件に一致するユーザーが[`MAX_FILTERED_USERS`]件より少なくなるように、検索語または外部識別子を指定する。
    ///
    /// # Arguments
    ///
    /// - `options`: 検索語、ユーザーの種類およびユーザー数の上限などのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXが以前に返したマーカーを再び返した場合、このメソッドは無限ループを避けるため
    /// [`BoxClientError::UnexpectedResponse`]を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みのユーザーを破棄してエラーを返す。
    pub async fn list_all_enterprise_users(
        &self,
        options: &ListEnterpriseUsersOptions<'_>,
    ) -> BoxClientResult<Vec<User>> {
        let options = ListEnterpriseUsersOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
            ..*options
        };
        let (users, pages) =
            collect_all_pages(async |marker| self.list_enterprise_users(marker, &options).await)
                .await?;

        tracing::debug!(pages, users = users.len(), "listed all enterprise users");

        Ok(users)
    }

    /// エンタープライズにユーザーを作成する。
    ///
    /// BOXのドキュメントによると、管理者の権限を持つユーザーまたはアプリケーションだけが実行できる。
    ///
    /// HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。
    /// 5xxまたはタイムアウトの後に再送すると、同じユーザーを重複して作成するおそれがあるためである。
    ///
    /// # Arguments
    ///
    /// - `name`: ユーザー名。1文字以上[`MAX_USER_NAME_CHARS`]文字以下で指定する。
    /// - `account`: 作成するユーザーの種類。管理対象ユーザーの場合は、ログインに使用するメールアドレスを指定する。
    /// - `options`: 役割、言語、役職およびストレージの容量などのオプション
    ///
    /// # Errors
    ///
    /// 管理対象ユーザーのメールアドレスが空文字列もしくは空白だけの場合、
    /// または`options.status`が[`UserStatus::Unknown`]の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// ユーザー名、役職、電話番号、住所またはストレージの容量をBOXが拒否する場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがユーザーを作成した可能性がある。
    /// したがって、[`BoxClient::list_enterprise_users`]でユーザーを確認する。
    pub async fn create_user(
        &self,
        name: &str,
        account: NewUserAccount<'_>,
        options: &CreateUserOptions<'_>,
    ) -> BoxClientResult<User> {
        let body = CreateUserBody::new(name, account, options)?;
        let request = ApiRequest::post(["users"]).json(&body)?;

        let user: User = self.send_json(request).await?;

        // ユーザー名とメールアドレスは個人情報であるため、ログに記録しない。
        tracing::debug!(
            user_id = user.id.as_ref(),
            app_user = matches!(account, NewUserAccount::AppUser),
            "created user"
        );

        Ok(user)
    }

    /// ユーザーを更新する。
    ///
    /// BOXのドキュメントによると、管理者の権限を持つユーザーまたはアプリケーションだけが、
    /// エンタープライズの管理対象ユーザーまたはアプリユーザーを更新できる。
    ///
    /// ユーザーをエンタープライズから除外する場合は、冪等でないリクエストとして送信する。
    /// 5xxまたはタイムアウトの後に再送すると、ユーザーがすでにエンタープライズに属しておらず、
    /// BOXがエラーを返すおそれがあるためである。
    /// それ以外の更新は、同じ値を何度設定しても結果が変わらないため、冪等なリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `user_id`: 更新するユーザーのID
    /// - `update`: 変更する属性
    ///
    /// # Errors
    ///
    /// メールアドレスもしくは通知を送信する代わりのメールアドレスが空文字列もしくは空白だけの場合、
    /// `update.status`が[`UserStatus::Unknown`]の場合、
    /// または`update.roll_out_of_enterprise`が`false`で`update.notify`を指定した場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// ユーザー名、役職、電話番号、住所またはストレージの容量をBOXが拒否する場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// BOXのドキュメントによると、BOXは次のエラーを返し、BOXクライアントは[`BoxClientError::Api`]を返す。
    ///
    /// - 400（`invalid_parameter`）: パラメーターの形式が正しくない場合
    /// - 400（`user_must_not_have_retained_content`）: リテンションポリシーまたはリーガルホールドのため、
    ///   ユーザーをエンタープライズから除外できない場合
    /// - 403（`access_denied_insufficient_permissions`）: ユーザーを更新する権限がない場合
    /// - 403（`denied_by_policy`）: 情報バリアによって更新を禁止している場合
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// エンタープライズからの除外が5xxまたはタイムアウトでエラーを返した場合は、BOXがユーザーを除外した可能性がある。
    /// したがって、[`BoxClient::user_information`]でユーザーを確認する。
    pub async fn update_user(
        &self,
        user_id: &UserId,
        update: &UserUpdate<'_>,
    ) -> BoxClientResult<User> {
        let body = UpdateUserBody::new(update)?;
        let mut request = ApiRequest::put(["users", user_id.as_ref()]);
        if update.roll_out_of_enterprise {
            request = request.idempotency(Idempotency::NonIdempotent);
        }
        let request = request.json(&body)?;

        let user: User = self.send_json(request).await?;

        tracing::debug!(
            user_id = user_id.as_ref(),
            rolled_out_of_enterprise = update.roll_out_of_enterprise,
            "updated user"
        );

        Ok(user)
    }

    /// ユーザーを削除する。
    ///
    /// BOXのドキュメントによると、既定ではユーザーがコンテンツを所有していると削除に失敗する。
    /// ユーザーが所有するコンテンツも削除する場合は、`options.force`に`true`を指定する。
    ///
    /// BOXクライアントは、冪等なリクエストであれば、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがユーザーの削除を完了している可能性がある。
    /// BOXが削除した後にリクエストを再送すると、削除済みのユーザーに対する削除となるため、BOXはエラーを返しうる。
    /// その結果、削除に成功したにもかかわらず、削除に失敗したと誤って判断するおそれがある。
    /// したがって、HTTPメソッドがDELETEであっても冪等でないリクエストとして送信し、
    /// BOXがリクエストを処理していないことが明らかな429、エラーコードが`operation_blocked_temporary`の409
    /// および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `user_id`: 削除するユーザーのID
    /// - `options`: 通知およびコンテンツを所有するユーザーを削除するかどうかのオプション
    ///
    /// # Errors
    ///
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。
    /// BOXのドキュメントによると、ユーザーがコンテンツを所有し、エンタープライズにリテンションポリシー（廃止したものを含む）
    /// または解除していないリーガルホールドがある場合、BOXはエラーコードが`forbidden`の403を返す。
    /// この場合は、`options.force`に`true`を指定しても削除できないため、ユーザーのコンテンツを移管してから削除する。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがユーザーを削除した可能性があるため、
    /// [`BoxClient::user_information`]でユーザーが存在するか確認する。
    pub async fn delete_user(
        &self,
        user_id: &UserId,
        options: &DeleteUserOptions,
    ) -> BoxClientResult<()> {
        let mut request =
            ApiRequest::delete(["users", user_id.as_ref()]).idempotency(Idempotency::NonIdempotent);
        if let Some(notify) = options.notify {
            request = request.query("notify", if notify { "true" } else { "false" });
        }
        if let Some(force) = options.force {
            request = request.query("force", if force { "true" } else { "false" });
        }

        self.send_empty(request).await?;

        tracing::debug!(user_id = user_id.as_ref(), "deleted user");

        Ok(())
    }
}

#[cfg(test)]
mod test_user {
    use rstest::rstest;
    use serde_json::json;
    use time::macros::datetime;

    use super::*;

    /// 標準の表現のすべての属性をデシリアライズすることを確認
    #[test]
    fn test_deserialize_user() -> anyhow::Result<()> {
        let user: User = serde_json::from_value(json!({
            "id": "11446498",
            "type": "user",
            "name": "Aaron Levie",
            "login": "ceo@example.com",
            "created_at": "2012-12-12T10:53:43-08:00",
            "modified_at": "2012-12-12T10:53:43-08:00",
            "language": "en",
            "timezone": "Africa/Bujumbura",
            "space_amount": 11_345_156_112_i64,
            "space_used": 1_237_009_912,
            "max_upload_size": 2_147_483_648_i64,
            "status": "active",
            "job_title": "CEO",
            "phone": "6509241374",
            "address": "900 Jefferson Ave, Redwood City, CA 94063",
            "avatar_url": "https://www.box.com/api/avatar/large/181216415",
            "notification_email": {
                "email": "notifications@example.com",
                "is_confirmed": true,
            },
        }))?;

        assert_eq!(user.id, UserId::new("11446498"));
        assert_eq!(user.user_type, "user");
        assert_eq!(user.name.as_deref(), Some("Aaron Levie"));
        assert_eq!(user.login.as_deref(), Some("ceo@example.com"));
        assert_eq!(user.created_at, Some(datetime!(2012-12-12 10:53:43 -08:00)));
        assert_eq!(
            user.modified_at,
            Some(datetime!(2012-12-12 10:53:43 -08:00))
        );
        assert_eq!(user.language.as_deref(), Some("en"));
        assert_eq!(user.timezone.as_deref(), Some("Africa/Bujumbura"));
        assert_eq!(user.space_amount, Some(11_345_156_112));
        assert_eq!(user.space_used, Some(1_237_009_912));
        assert_eq!(user.max_upload_size, Some(2_147_483_648));
        assert_eq!(user.status, Some(UserStatus::Active));
        assert_eq!(user.job_title.as_deref(), Some("CEO"));
        assert_eq!(user.phone.as_deref(), Some("6509241374"));
        assert_eq!(
            user.address.as_deref(),
            Some("900 Jefferson Ave, Redwood City, CA 94063")
        );
        assert_eq!(
            user.avatar_url.as_deref(),
            Some("https://www.box.com/api/avatar/large/181216415")
        );
        assert_eq!(
            user.notification_email,
            Some(NotificationEmail {
                email: Some("notifications@example.com".to_owned()),
                is_confirmed: Some(true),
            })
        );

        Ok(())
    }

    /// `id`と`type`以外の属性が省略またはnullの場合も、`None`としてデシリアライズすることを確認
    ///
    /// BOXは、外部ユーザーの情報で、取得を許可しない属性をnullで返す。
    #[rstest]
    #[case::missing(json!({"id": "1", "type": "user"}))]
    #[case::null(json!({
        "id": "1",
        "type": "user",
        "name": null,
        "login": null,
        "created_at": null,
        "modified_at": null,
        "language": null,
        "timezone": null,
        "space_amount": null,
        "space_used": null,
        "max_upload_size": null,
        "status": null,
        "job_title": null,
        "phone": null,
        "address": null,
        "avatar_url": null,
        "notification_email": null,
    }))]
    fn test_deserialize_user_without_attributes(
        #[case] value: serde_json::Value,
    ) -> anyhow::Result<()> {
        let user: User = serde_json::from_value(value)?;

        assert_eq!(user.id, UserId::new("1"));
        assert_eq!(user.name, None);
        assert_eq!(user.created_at, None);
        assert_eq!(user.space_amount, None);
        assert_eq!(user.status, None);
        assert_eq!(user.notification_email, None);

        Ok(())
    }

    /// アカウントの状態を、BOXクライアントが知らない状態も含めてデシリアライズすることを確認
    #[rstest]
    #[case::active("active", UserStatus::Active)]
    #[case::inactive("inactive", UserStatus::Inactive)]
    #[case::cannot_delete_edit("cannot_delete_edit", UserStatus::CannotDeleteEdit)]
    #[case::cannot_delete_edit_upload(
        "cannot_delete_edit_upload",
        UserStatus::CannotDeleteEditUpload
    )]
    #[case::unknown("suspended", UserStatus::Unknown)]
    fn test_deserialize_user_status(
        #[case] value: &str,
        #[case] expected: UserStatus,
    ) -> anyhow::Result<()> {
        let status: UserStatus = serde_json::from_value(json!(value))?;

        assert_eq!(status, expected);

        Ok(())
    }
}

#[cfg(test)]
mod test_get_current_user {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use crate::test_support::{create_client, mount_token, short_retry};

    /// `GET /users/me`を呼び出し、ユーザー情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_get_current_user() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/users/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "user",
                "id": "11446498",
                "name": "Service Account",
                "login": "AutomationUser_123@boxdevedition.com",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let user = client.me().await?;

        assert_eq!(user.id, crate::UserId::new("11446498"));
        assert_eq!(user.name.as_deref(), Some("Service Account"));
        assert_eq!(
            user.login.as_deref(),
            Some("AutomationUser_123@boxdevedition.com")
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_user_information {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// `GET /users/{user_id}`を呼び出し、ユーザー情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_user_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/users/12345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "user",
                "id": "12345",
                "name": "Test User",
                "login": "user@example.com",
                "status": "inactive",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let user = client.user_information(&UserId::new("12345")).await?;

        assert_eq!(user.id, UserId::new("12345"));
        assert_eq!(user.name.as_deref(), Some("Test User"));
        assert_eq!(user.status, Some(UserStatus::Inactive));

        Ok(())
    }
}

#[cfg(test)]
mod test_list_enterprise_users {
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

    /// ユーザーのAPIのパス
    const USERS_PATH: &str = "/2.0/users";

    /// 検索語と外部識別子を指定しない場合は、マーカー方式でマーカーとオプションを送信し、
    /// BOXが返した`next_marker`をそのまま返すことを確認
    #[rstest]
    #[case::default(None, ListEnterpriseUsersOptions::default(), "usemarker=true")]
    #[case::marker_and_options(
        Some("m1"),
        ListEnterpriseUsersOptions::default()
            .with_user_type(UserTypeFilter::Managed)
            .with_limit(100),
        "usemarker=true&limit=100&marker=m1&user_type=managed"
    )]
    #[case::all_user_type(
        None,
        ListEnterpriseUsersOptions::default().with_user_type(UserTypeFilter::All),
        "usemarker=true&user_type=all"
    )]
    #[case::external_user_type(
        None,
        ListEnterpriseUsersOptions::default().with_user_type(UserTypeFilter::External),
        "usemarker=true&user_type=external"
    )]
    #[tokio::test]
    async fn test_list_enterprise_users_with_marker(
        #[case] marker: Option<&str>,
        #[case] options: ListEnterpriseUsersOptions<'_>,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(USERS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [
                    {"type": "user", "id": "1", "name": "John Doe"},
                    {"type": "user", "id": "2", "name": "John Smith"},
                ],
                "limit": 100,
                "next_marker": "m2",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client.list_enterprise_users(marker, &options).await?;

        let ids: Vec<_> = page.entries.iter().map(|u| u.id.as_ref()).collect();
        assert_eq!(ids, vec!["1", "2"]);
        assert_eq!(page.limit, Some(100));
        assert_eq!(page.next_marker.as_deref(), Some("m2"));
        assert_eq!(
            request_queries(&server, USERS_PATH).await,
            vec![expected_query]
        );

        Ok(())
    }

    /// 検索語または外部識別子を指定した場合は、マーカーをオフセットとしてオフセット方式で送信することを確認
    #[rstest]
    #[case::filter_term_first_page(
        None,
        ListEnterpriseUsersOptions::default().with_filter_term("john"),
        "offset=0&filter_term=john"
    )]
    #[case::all_options(
        Some("2"),
        ListEnterpriseUsersOptions::default()
            .with_filter_term("john")
            .with_user_type(UserTypeFilter::Managed)
            .with_external_app_user_id("my-user-1234")
            .with_limit(100),
        "offset=2&limit=100&filter_term=john&user_type=managed&external_app_user_id=my-user-1234"
    )]
    #[case::external_app_user_id(
        None,
        ListEnterpriseUsersOptions::default().with_external_app_user_id("my-user-1234"),
        "offset=0&external_app_user_id=my-user-1234"
    )]
    #[case::last_offset_below_max(
        Some("9999"),
        ListEnterpriseUsersOptions::default().with_filter_term("john"),
        "offset=9999&filter_term=john"
    )]
    #[tokio::test]
    async fn test_list_enterprise_users_with_offset(
        #[case] marker: Option<&str>,
        #[case] options: ListEnterpriseUsersOptions<'_>,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(USERS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "user", "id": "1"}],
                "total_count": 20_000,
                "limit": 100,
                "offset": 0,
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client.list_enterprise_users(marker, &options).await?;

        let ids: Vec<_> = page.entries.iter().map(|u| u.id.as_ref()).collect();
        assert_eq!(ids, vec!["1"]);
        assert_eq!(page.limit, Some(100));
        assert_eq!(
            request_queries(&server, USERS_PATH).await,
            vec![expected_query]
        );

        Ok(())
    }

    /// オフセット方式では、要求したオフセットとエントリー数から次のページのオフセットを求め、
    /// `total_count`もしくは取得できるユーザー数の上限に達した場合、または空のページの場合は
    /// `next_marker`を`None`にすることを確認
    ///
    /// 上限を超えるユーザーは、ページから取り除く。
    #[rstest]
    #[case::has_next(None, 2, 5, 2, Some("2"))]
    #[case::has_next_from_offset(Some("2"), 2, 5, 2, Some("4"))]
    #[case::reaches_total_count(Some("4"), 1, 5, 1, None)]
    #[case::empty_page(Some("2"), 0, 5, 0, None)]
    #[case::reaches_max(Some("9998"), 2, 20_000, 2, None)]
    #[case::truncated_at_max(Some("9999"), 2, 20_000, 1, None)]
    #[tokio::test]
    async fn test_list_enterprise_users_with_offset_next_marker(
        #[case] marker: Option<&str>,
        #[case] entries: usize,
        #[case] total_count: u64,
        #[case] expected_entries: usize,
        #[case] expected_next_marker: Option<&str>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let entries: Vec<_> = (0..entries)
            .map(|i| json!({"type": "user", "id": i.to_string()}))
            .collect();
        Mock::given(method("GET"))
            .and(path(USERS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": entries,
                "total_count": total_count,
                "limit": 2,
                "offset": 0,
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client
            .list_enterprise_users(
                marker,
                &ListEnterpriseUsersOptions::default()
                    .with_filter_term("kato")
                    .with_limit(2),
            )
            .await?;

        assert_eq!(page.entries.len(), expected_entries);
        assert_eq!(page.next_marker.as_deref(), expected_next_marker);

        Ok(())
    }

    /// BOXが`limit`と`next_marker`を含めずに返したレスポンスでも、ページをデシリアライズすることを確認
    #[tokio::test]
    async fn test_list_enterprise_users_without_limit() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(USERS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"entries": []})))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let page = client
            .list_enterprise_users(None, &ListEnterpriseUsersOptions::default())
            .await?;

        assert_eq!(page.entries, Vec::new());
        assert_eq!(page.limit, None);
        assert_eq!(page.next_marker, None);

        Ok(())
    }

    /// 上限、またはオフセット方式のマーカーが不正な場合は、リクエストを送信せずに`InvalidArgument`を返すことを確認
    #[rstest]
    #[case::zero_limit(None, ListEnterpriseUsersOptions::default().with_limit(0))]
    #[case::limit_above_max(
        None,
        ListEnterpriseUsersOptions::default().with_limit(MAX_ITEMS_LIMIT + 1)
    )]
    #[case::zero_limit_with_filter(
        None,
        ListEnterpriseUsersOptions::default().with_filter_term("john").with_limit(0)
    )]
    #[case::marker_not_offset(
        Some("m1"),
        ListEnterpriseUsersOptions::default().with_filter_term("john")
    )]
    #[case::negative_offset(
        Some("-1"),
        ListEnterpriseUsersOptions::default().with_external_app_user_id("my-user-1234")
    )]
    #[case::offset_at_max(
        Some("10000"),
        ListEnterpriseUsersOptions::default().with_filter_term("john")
    )]
    #[case::offset_above_max(
        Some("10001"),
        ListEnterpriseUsersOptions::default().with_filter_term("john")
    )]
    #[tokio::test]
    async fn test_list_enterprise_users_invalid_arguments(
        #[case] marker: Option<&str>,
        #[case] options: ListEnterpriseUsersOptions<'_>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = client.list_enterprise_users(marker, &options).await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }
}

#[cfg(test)]
mod test_list_all_enterprise_users {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// ユーザーのAPIのパス
    const USERS_PATH: &str = "/2.0/users";

    /// 検索語と外部識別子を指定しない場合は、`next_marker`をたどってすべてのページを取得し、
    /// 上限の既定値とオプションを各ページで送信することを確認
    #[tokio::test]
    async fn test_list_all_enterprise_users_with_marker() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(USERS_PATH))
            .and(query_param("usemarker", "true"))
            .and(query_param("limit", "1000"))
            .and(query_param("user_type", "managed"))
            .and(query_param_is_missing("marker"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "user", "id": "1"}],
                "limit": 1000,
                "next_marker": "m2",
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(USERS_PATH))
            .and(query_param("usemarker", "true"))
            .and(query_param("limit", "1000"))
            .and(query_param("user_type", "managed"))
            .and(query_param("marker", "m2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "user", "id": "2"}],
                "limit": 1000,
                "next_marker": null,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let users = client
            .list_all_enterprise_users(
                &ListEnterpriseUsersOptions::default().with_user_type(UserTypeFilter::Managed),
            )
            .await?;

        let ids: Vec<_> = users.iter().map(|u| u.id.as_ref()).collect();
        assert_eq!(ids, vec!["1", "2"]);

        Ok(())
    }

    /// 検索語を指定した場合は、オフセットを進めながらすべてのページを取得し、
    /// `total_count`に達したら終了することを確認
    #[tokio::test]
    async fn test_list_all_enterprise_users_with_offset() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(USERS_PATH))
            .and(query_param("offset", "0"))
            .and(query_param("limit", "2"))
            .and(query_param("filter_term", "kato"))
            .and(query_param_is_missing("usemarker"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "user", "id": "1"}, {"type": "user", "id": "2"}],
                "total_count": 3,
                "limit": 2,
                "offset": 0,
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(USERS_PATH))
            .and(query_param("offset", "2"))
            .and(query_param("limit", "2"))
            .and(query_param("filter_term", "kato"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "user", "id": "3"}],
                "total_count": 3,
                "limit": 2,
                "offset": 2,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let users = client
            .list_all_enterprise_users(
                &ListEnterpriseUsersOptions::default()
                    .with_filter_term("kato")
                    .with_limit(2),
            )
            .await?;

        let ids: Vec<_> = users.iter().map(|u| u.id.as_ref()).collect();
        assert_eq!(ids, vec!["1", "2", "3"]);

        Ok(())
    }

    /// 検索語に一致するユーザーが上限より多い場合は、エラーにせず、先頭から上限の件数までを返すことを確認
    ///
    /// 上限が1ページの件数で割り切れない場合も、最後のページの余分なユーザーを取り除き、上限ちょうどの件数を返す。
    #[rstest]
    #[case::limit_divides_max(1000, 10)]
    #[case::limit_does_not_divide_max(300, 34)]
    #[tokio::test]
    async fn test_list_all_enterprise_users_with_offset_stops_at_max(
        #[case] limit: u32,
        #[case] expected_requests: usize,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(USERS_PATH))
            .respond_with(move |request: &wiremock::Request| {
                let offset: u64 = request
                    .url
                    .query_pairs()
                    .find(|(key, _)| key == "offset")
                    .and_then(|(_, value)| value.parse().ok())
                    .unwrap_or_default();
                let entries: Vec<_> = (offset..offset + u64::from(limit))
                    .map(|id| json!({"type": "user", "id": id.to_string()}))
                    .collect();
                ResponseTemplate::new(200).set_body_json(json!({
                    "entries": entries,
                    "total_count": 20_000,
                    "limit": limit,
                    "offset": offset,
                }))
            })
            .expect(u64::try_from(expected_requests)?)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let users = client
            .list_all_enterprise_users(
                &ListEnterpriseUsersOptions::default()
                    .with_filter_term("a")
                    .with_limit(limit),
            )
            .await?;

        assert_eq!(users.len() as u64, MAX_FILTERED_USERS);
        assert_eq!(users.first().map(|u| u.id.as_ref()), Some("0"));
        assert_eq!(users.last().map(|u| u.id.as_ref()), Some("9999"));

        Ok(())
    }

    /// 上限を指定した場合は、その値を各ページで送信することを確認
    #[tokio::test]
    async fn test_list_all_enterprise_users_with_limit() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(USERS_PATH))
            .and(query_param("limit", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "user", "id": "1"}],
                "next_marker": null,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let users = client
            .list_all_enterprise_users(&ListEnterpriseUsersOptions::default().with_limit(10))
            .await?;

        assert_eq!(users.len(), 1);

        Ok(())
    }
}

#[cfg(test)]
mod test_create_user {
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

    /// ユーザーのAPIのパス
    const USERS_PATH: &str = "/2.0/users";

    /// テストで使用するトラッキングコード
    const TRACKING_CODES: &[TrackingCode<'static>] = &[TrackingCode {
        name: "department",
        value: "Sales",
    }];

    /// 作成したユーザーのレスポンスを返す。
    fn created_response() -> ResponseTemplate {
        ResponseTemplate::new(201).set_body_json(json!({
            "type": "user",
            "id": "999",
            "name": "Test User",
        }))
    }

    /// 名前、ユーザーの種類およびオプションをリクエストボディに変換し、作成したユーザーを返すことを確認
    #[rstest]
    // 管理対象ユーザーを最小限の指定で作成する
    #[case::managed_minimal(
        NewUserAccount::Managed { login: "user@example.com" },
        CreateUserOptions::default(),
        json!({"name": "Test User", "login": "user@example.com"})
    )]
    // アプリユーザーを、すべてのオプションを指定して作成する
    #[case::app_user_all_options(
        NewUserAccount::AppUser,
        CreateUserOptions::default()
            .with_role(UserRole::Coadmin)
            .with_language("ja")
            .with_timezone("Asia/Tokyo")
            .with_is_sync_enabled(true)
            .with_job_title("CEO")
            .with_phone("6509241374")
            .with_address("900 Jefferson Ave")
            .with_space_amount(UNLIMITED_SPACE_AMOUNT)
            .with_tracking_codes(TRACKING_CODES)
            .with_can_see_managed_users(false)
            .with_is_external_collab_restricted(true)
            .with_is_exempt_from_device_limits(false)
            .with_is_exempt_from_login_verification(false)
            .with_status(UserStatus::CannotDeleteEdit)
            .with_external_app_user_id("my-user-1234"),
        json!({
            "name": "Test User",
            "is_platform_access_only": true,
            "role": "coadmin",
            "language": "ja",
            "timezone": "Asia/Tokyo",
            "is_sync_enabled": true,
            "job_title": "CEO",
            "phone": "6509241374",
            "address": "900 Jefferson Ave",
            "space_amount": -1,
            "tracking_codes": [{"type": "tracking_code", "name": "department", "value": "Sales"}],
            "can_see_managed_users": false,
            "is_external_collab_restricted": true,
            "is_exempt_from_device_limits": false,
            "is_exempt_from_login_verification": false,
            "status": "cannot_delete_edit",
            "external_app_user_id": "my-user-1234",
        })
    )]
    #[tokio::test]
    async fn test_create_user(
        #[case] account: NewUserAccount<'_>,
        #[case] options: CreateUserOptions<'_>,
        #[case] expected_body: serde_json::Value,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(USERS_PATH))
            .and(body_json(expected_body))
            .respond_with(created_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let user = client.create_user("Test User", account, &options).await?;

        assert_eq!(user.id, UserId::new("999"));

        Ok(())
    }

    /// 引数またはオプションをBOXが拒否する場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::empty_name(
        "",
        NewUserAccount::AppUser,
        CreateUserOptions::default(),
        is_invalid_request_body
    )]
    #[case::too_long_name(
        &"a".repeat(MAX_USER_NAME_CHARS + 1),
        NewUserAccount::AppUser,
        CreateUserOptions::default(),
        is_invalid_request_body
    )]
    #[case::empty_login(
        "Test User",
        NewUserAccount::Managed { login: " " },
        CreateUserOptions::default(),
        is_invalid_argument
    )]
    #[case::too_long_job_title(
        "Test User",
        NewUserAccount::AppUser,
        CreateUserOptions::default().with_job_title("a".repeat(MAX_USER_JOB_TITLE_CHARS + 1).leak()),
        is_invalid_request_body
    )]
    #[case::too_long_phone(
        "Test User",
        NewUserAccount::AppUser,
        CreateUserOptions::default().with_phone("1".repeat(MAX_USER_PHONE_CHARS + 1).leak()),
        is_invalid_request_body
    )]
    #[case::too_long_address(
        "Test User",
        NewUserAccount::AppUser,
        CreateUserOptions::default().with_address("a".repeat(MAX_USER_ADDRESS_CHARS + 1).leak()),
        is_invalid_request_body
    )]
    #[case::space_amount_below_unlimited(
        "Test User",
        NewUserAccount::AppUser,
        CreateUserOptions::default().with_space_amount(-2),
        is_invalid_request_body
    )]
    #[case::unknown_status(
        "Test User",
        NewUserAccount::AppUser,
        CreateUserOptions::default().with_status(UserStatus::Unknown),
        is_invalid_argument
    )]
    #[tokio::test]
    async fn test_create_user_invalid_arguments(
        #[case] name: &str,
        #[case] account: NewUserAccount<'_>,
        #[case] options: CreateUserOptions<'_>,
        #[case] is_expected_error: fn(&BoxClientError) -> bool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = client.create_user(name, account, &options).await;

        let Err(error) = result else {
            panic!("result: {result:?}");
        };
        assert!(is_expected_error(&error), "error: {error:?}");
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// 上限の文字数ちょうどの名前、役職、電話番号および住所を受け付けることを確認
    #[tokio::test]
    async fn test_create_user_accepts_max_lengths() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(USERS_PATH))
            .respond_with(created_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;
        let (name, job_title, phone, address) = (
            "あ".repeat(MAX_USER_NAME_CHARS),
            "あ".repeat(MAX_USER_JOB_TITLE_CHARS),
            "1".repeat(MAX_USER_PHONE_CHARS),
            "あ".repeat(MAX_USER_ADDRESS_CHARS),
        );

        client
            .create_user(
                &name,
                NewUserAccount::AppUser,
                &CreateUserOptions::default()
                    .with_job_title(&job_title)
                    .with_phone(&phone)
                    .with_address(&address),
            )
            .await?;

        Ok(())
    }

    /// ユーザーを重複して作成しないように、5xxを受け取っても再試行しないことを確認
    #[rstest]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_create_user_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        assert_not_retried("POST", USERS_PATH, status, async |client: &BoxClient| {
            client
                .create_user(
                    "Test User",
                    NewUserAccount::AppUser,
                    &CreateUserOptions::default(),
                )
                .await
        })
        .await
    }
}

#[cfg(test)]
mod test_update_user {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_not_retried, assert_retried_after, create_client, is_invalid_argument,
        is_invalid_request_body, mount_token, request_count, short_retry,
    };

    /// ユーザーのAPIのパス
    const USER_PATH: &str = "/2.0/users/12345";

    /// テストで使用するトラッキングコード
    const TRACKING_CODES: &[TrackingCode<'static>] = &[TrackingCode {
        name: "department",
        value: "Sales",
    }];

    /// 更新したユーザーのレスポンスを返す。
    fn updated_response() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "type": "user",
            "id": "12345",
            "name": "Updated User",
        }))
    }

    /// 変更する属性をリクエストボディに変換し、更新したユーザーを返すことを確認
    #[rstest]
    // 何も変更しない
    #[case::empty(UserUpdate::default(), json!({}))]
    // すべての属性を変更し、通知を送信する代わりのメールアドレスを設定する
    #[case::all_attributes(
        UserUpdate::default()
            .with_name("Updated User")
            .with_login("new@example.com")
            .with_role(UserRole::User)
            .with_language("en")
            .with_timezone("UTC")
            .with_is_sync_enabled(false)
            .with_job_title("Manager")
            .with_phone("0123456789")
            .with_address("Tokyo")
            .with_space_amount(1_073_741_824)
            .with_tracking_codes(TRACKING_CODES)
            .with_can_see_managed_users(true)
            .with_is_external_collab_restricted(false)
            .with_is_exempt_from_device_limits(true)
            .with_is_exempt_from_login_verification(true)
            .with_is_password_reset_required(true)
            .with_is_collaborated_content_available_when_owner_inactive(true)
            .with_status(UserStatus::Inactive)
            .with_notification_email("notify@example.com")
            .with_external_app_user_id("my-user-5678"),
        json!({
            "name": "Updated User",
            "login": "new@example.com",
            "role": "user",
            "language": "en",
            "timezone": "UTC",
            "is_sync_enabled": false,
            "job_title": "Manager",
            "phone": "0123456789",
            "address": "Tokyo",
            "space_amount": 1_073_741_824,
            "tracking_codes": [{"type": "tracking_code", "name": "department", "value": "Sales"}],
            "can_see_managed_users": true,
            "is_external_collab_restricted": false,
            "is_exempt_from_device_limits": true,
            "is_exempt_from_login_verification": true,
            "is_password_reset_required": true,
            "is_collaborated_content_available_when_owner_inactive": true,
            "status": "inactive",
            "notification_email": {"email": "notify@example.com"},
            "external_app_user_id": "my-user-5678",
        })
    )]
    // 通知を送信する代わりのメールアドレスを削除する
    #[case::notification_email_removed(
        UserUpdate::default().with_notification_email_removed(),
        json!({"notification_email": null})
    )]
    // エンタープライズから除外して、ユーザーに通知する
    #[case::roll_out_of_enterprise(
        UserUpdate::default().with_roll_out_of_enterprise().with_notify(true),
        json!({"enterprise": null, "notify": true})
    )]
    #[tokio::test]
    async fn test_update_user(
        #[case] update: UserUpdate<'_>,
        #[case] expected_body: serde_json::Value,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(USER_PATH))
            .and(body_json(expected_body))
            .respond_with(updated_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        let user = client.update_user(&UserId::new("12345"), &update).await?;

        assert_eq!(user.id, UserId::new("12345"));
        assert_eq!(user.name.as_deref(), Some("Updated User"));

        Ok(())
    }

    /// 変更する属性をBOXが拒否する場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::empty_name(UserUpdate::default().with_name(""), is_invalid_request_body)]
    #[case::too_long_name(
        UserUpdate::default().with_name("a".repeat(MAX_USER_NAME_CHARS + 1).leak()),
        is_invalid_request_body
    )]
    #[case::empty_login(UserUpdate::default().with_login(""), is_invalid_argument)]
    #[case::empty_notification_email(
        UserUpdate::default().with_notification_email(" "),
        is_invalid_argument
    )]
    #[case::too_long_job_title(
        UserUpdate::default().with_job_title("a".repeat(MAX_USER_JOB_TITLE_CHARS + 1).leak()),
        is_invalid_request_body
    )]
    #[case::space_amount_below_unlimited(
        UserUpdate::default().with_space_amount(-2),
        is_invalid_request_body
    )]
    #[case::unknown_status(
        UserUpdate::default().with_status(UserStatus::Unknown),
        is_invalid_argument
    )]
    #[case::notify_without_roll_out(UserUpdate::default().with_notify(true), is_invalid_argument)]
    #[tokio::test]
    async fn test_update_user_invalid_arguments(
        #[case] update: UserUpdate<'_>,
        #[case] is_expected_error: fn(&BoxClientError) -> bool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)).await?;
        let requests_before = request_count(&server).await;

        let result = client.update_user(&UserId::new("12345"), &update).await;

        let Err(error) = result else {
            panic!("result: {result:?}");
        };
        assert!(is_expected_error(&error), "error: {error:?}");
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// エンタープライズからの除外以外の更新は、冪等なリクエストとして5xxの後に再試行することを確認
    #[tokio::test]
    async fn test_update_user_retries_after_server_error() -> anyhow::Result<()> {
        assert_retried_after(
            "PUT",
            USER_PATH,
            ResponseTemplate::new(500),
            updated_response(),
            async |client: &BoxClient| {
                client
                    .update_user(
                        &UserId::new("12345"),
                        &UserUpdate::default().with_job_title("Manager"),
                    )
                    .await
            },
        )
        .await?;

        Ok(())
    }

    /// エンタープライズからの除外は、冪等でないリクエストとして5xxの後に再試行しないことを確認
    #[rstest]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_update_user_roll_out_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        assert_not_retried("PUT", USER_PATH, status, async |client: &BoxClient| {
            client
                .update_user(
                    &UserId::new("12345"),
                    &UserUpdate::default().with_roll_out_of_enterprise(),
                )
                .await
        })
        .await
    }
}

#[cfg(test)]
mod test_delete_user {
    use rstest::rstest;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_not_retried, assert_retried_after, create_client, mount_token,
        operation_blocked_response, request_queries, short_retry,
    };

    /// ユーザーのAPIのパス
    const USER_PATH: &str = "/2.0/users/12345";

    /// `DELETE /users/{user_id}`をオプションのクエリパラメーター付きで呼び出し、204を受け取ると成功することを確認
    #[rstest]
    #[case::default(DeleteUserOptions::default(), "")]
    #[case::notify_and_force(
        DeleteUserOptions::default().with_notify(false).with_force(true),
        "notify=false&force=true"
    )]
    #[case::notify_true_force_false(
        DeleteUserOptions::default().with_notify(true).with_force(false),
        "notify=true&force=false"
    )]
    #[tokio::test]
    async fn test_delete_user(
        #[case] options: DeleteUserOptions,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(USER_PATH))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)).await?;

        client.delete_user(&UserId::new("12345"), &options).await?;

        assert_eq!(
            request_queries(&server, USER_PATH).await,
            vec![expected_query]
        );

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 5xxの後に再送すると、削除済みのユーザーに対してエラーを受け取るおそれがあるため、
    /// 再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::forbidden(403)]
    #[case::not_found(404)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_delete_user_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        assert_not_retried("DELETE", USER_PATH, status, async |client: &BoxClient| {
            client
                .delete_user(&UserId::new("12345"), &DeleteUserOptions::default())
                .await
        })
        .await
    }

    /// 429または他の操作によるロックで409を受け取った場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[rstest]
    #[case::too_many_requests(ResponseTemplate::new(429))]
    #[case::operation_blocked_temporary(operation_blocked_response())]
    #[tokio::test]
    async fn test_delete_user_retries_request_not_processed(
        #[case] first_response: ResponseTemplate,
    ) -> anyhow::Result<()> {
        assert_retried_after(
            "DELETE",
            USER_PATH,
            first_response,
            ResponseTemplate::new(204),
            async |client: &BoxClient| {
                client
                    .delete_user(&UserId::new("12345"), &DeleteUserOptions::default())
                    .await
            },
        )
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod test_user_mini {
    use rstest::rstest;
    use serde_json::json;

    use super::*;

    /// 表示名とメールアドレスが省略またはnullの場合も、`None`としてデシリアライズすることを確認
    #[rstest]
    #[case::present(
        json!({"type": "user", "id": "1", "name": "Test User", "login": "user@example.com"}),
        Some("Test User"),
        Some("user@example.com")
    )]
    #[case::missing(json!({"type": "user", "id": "1"}), None, None)]
    #[case::null(json!({"type": "user", "id": "1", "name": null, "login": null}), None, None)]
    fn test_deserialize_user_mini(
        #[case] value: serde_json::Value,
        #[case] name: Option<&str>,
        #[case] login: Option<&str>,
    ) -> anyhow::Result<()> {
        let user: UserMini = serde_json::from_value(value)?;

        assert_eq!(user.id, crate::UserId::new("1"));
        assert_eq!(user.name.as_deref(), name);
        assert_eq!(user.login.as_deref(), login);

        Ok(())
    }
}
