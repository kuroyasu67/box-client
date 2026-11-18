//! ファイル、フォルダおよびWebリンクの情報の更新で共有する型
//!
//! BOXは、ファイルの更新（`PUT /files/{file_id}`）、フォルダの更新（`PUT /folders/{folder_id}`）および
//! Webリンクの更新（`PUT /web_links/{web_link_id}`）で、共有リンクなどの属性を同じ形式で受け付ける。

use serde::Serialize;
use time::OffsetDateTime;

use crate::{
    BoxClientError, BoxClientResult,
    api::types::{RenameBody, SetDescriptionBody},
};

/// 1つの項目に設定できるタグの最大数
///
/// BOXは、ファイルおよびフォルダのタグを1個以上100個以下で受け付ける。
pub const MAX_TAGS: usize = 100;

/// 更新する属性に指定する値、またはnull
///
/// BOXは、共有リンクやロックなどの一部の属性で、nullを受け取ると属性を削除する。
/// 属性を更新しない場合は、この型を包む`Option`を`None`にする。
///
/// # Examples
///
/// ```
/// use box_client::{FileUpdate, Nullable};
///
/// // 共有リンクを削除する。
/// let update = FileUpdate::default().with_shared_link_removed();
/// assert_eq!(update.shared_link, Some(Nullable::Null));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nullable<T> {
    /// 属性に設定する値
    Value(T),

    /// null
    Null,
}

impl<T> Nullable<T> {
    /// 値をリクエストボディの値に変換する。
    pub(crate) fn to_body<U>(&self, f: impl FnOnce(&T) -> U) -> NullableBody<U> {
        match self {
            Self::Value(value) => NullableBody::Value(f(value)),
            Self::Null => NullableBody::Null,
        }
    }
}

/// リクエストボディで、値またはnullを送信する属性
///
/// [`NullableBody::Null`]はnullにシリアライズする。属性を送信しない場合は、この型を包む`Option`を`None`にし、
/// `#[serde(skip_serializing_if = "Option::is_none")]`で省略する。
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum NullableBody<T> {
    /// 値
    Value(T),

    /// null
    Null,
}

/// 共有リンクにアクセスできるユーザーの範囲
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SharedLinkAccess {
    /// 共有リンクを知っているすべてのユーザー
    Open,

    /// エンタープライズのユーザー
    ///
    /// BOXは、有料のアカウントでだけ受け付ける。
    Company,

    /// 項目のコラボレーター
    Collaborators,
}

impl SharedLinkAccess {
    /// リクエストボディに指定する文字列を返す。
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Company => "company",
            Self::Collaborators => "collaborators",
        }
    }
}

/// 共有リンクのワンタイムパスコードの設定
///
/// ワンタイムパスコードを有効にすると、BOXは、共有リンクを開く匿名のユーザーに、
/// メールで送信したパスコードの入力を求める。
/// BOXは、アクセスできる範囲が[`SharedLinkAccess::Open`]の共有リンクでだけワンタイムパスコードを受け付け、
/// パスワードとワンタイムパスコードの併用を受け付けない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OneTimePasscode<'a> {
    /// ワンタイムパスコードを要求しない。
    Disabled,

    /// 任意のメールアドレスにパスコードを送信する。
    Open,

    /// 許可リストに一致するメールアドレスにだけパスコードを送信する。
    ///
    /// 指定した許可リストで、既存の許可リスト全体を置き換える。空のスライスを指定すると、許可リストを空にする。
    Allowlist(&'a [OtpAllowlistEntry<'a>]),
}

/// ワンタイムパスコードの許可リストの項目
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OtpAllowlistEntry<'a> {
    /// `@`を含むメールアドレス
    Email(&'a str),

    /// `example.com`のようなドメイン
    Domain(&'a str),
}

/// 作成または更新する共有リンクの設定
///
/// すべての属性が`None`の場合、BOXは、エンタープライズの既定の設定で共有リンクを作成する。
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// # Examples
///
/// ```
/// use box_client::{SharedLinkAccess, SharedLinkSettings};
///
/// let settings = SharedLinkSettings::default()
///     .with_access(SharedLinkAccess::Open)
///     .with_password("Secret-Passw0rd")
///     .with_can_download(false);
/// assert_eq!(settings.access, Some(SharedLinkAccess::Open));
/// ```
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SharedLinkSettings<'a> {
    /// 共有リンクにアクセスできるユーザーの範囲
    ///
    /// `None`の場合、BOXはエンタープライズの既定の範囲を使用する。
    pub access: Option<SharedLinkAccess>,

    /// 共有リンクのパスワード
    ///
    /// [`Nullable::Null`]を指定すると、パスワードを削除する。
    /// BOXは、アクセスできる範囲が[`SharedLinkAccess::Open`]の共有リンクでだけパスワードを受け付ける。
    /// また、BOXは、8文字以上で、数字、大文字または英数字以外の文字を含むパスワードだけを受け付ける。
    pub password: Option<Nullable<&'a str>>,

    /// 共有リンクのURLに使用する独自の文字列（バニティ名）
    ///
    /// BOXは、Webリンクの共有リンクで、12文字以上のバニティ名だけを受け付ける。
    /// バニティ名を使用したURLは推測されやすいため、BOXは機密性の高い内容に使用しないよう案内している。
    pub vanity_name: Option<&'a str>,

    /// 共有リンクを無効にする日時
    ///
    /// BOXは、有料のアカウントでだけ受け付ける。
    pub unshared_at: Option<OffsetDateTime>,

    /// 共有リンクから項目をダウンロードできるかどうか
    ///
    /// BOXは、アクセスできる範囲が[`SharedLinkAccess::Open`]または[`SharedLinkAccess::Company`]の
    /// 共有リンクでだけ受け付ける。Webリンクの共有リンクでは指定できない。
    pub can_download: Option<bool>,

    /// ワンタイムパスコードの設定
    pub one_time_passcode: Option<OneTimePasscode<'a>>,
}

impl std::fmt::Debug for SharedLinkSettings<'_> {
    /// パスワードをログやデバッグ出力に残さないように、パスワードを伏せて出力する。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let password = self.password.map(|password| match password {
            Nullable::Value(_) => Nullable::Value("[REDACTED]"),
            Nullable::Null => Nullable::Null,
        });
        f.debug_struct("SharedLinkSettings")
            .field("access", &self.access)
            .field("password", &password)
            .field("vanity_name", &self.vanity_name)
            .field("unshared_at", &self.unshared_at)
            .field("can_download", &self.can_download)
            .field("one_time_passcode", &self.one_time_passcode)
            .finish()
    }
}

impl<'a> SharedLinkSettings<'a> {
    /// 共有リンクにアクセスできるユーザーの範囲を設定する。
    #[must_use]
    pub fn with_access(mut self, access: SharedLinkAccess) -> Self {
        self.access = Some(access);
        self
    }

    /// 共有リンクのパスワードを設定する。
    #[must_use]
    pub fn with_password(mut self, password: &'a str) -> Self {
        self.password = Some(Nullable::Value(password));
        self
    }

    /// 共有リンクのパスワードを削除する。
    #[must_use]
    pub fn with_password_removed(mut self) -> Self {
        self.password = Some(Nullable::Null);
        self
    }

    /// 共有リンクのURLに使用する独自の文字列（バニティ名）を設定する。
    #[must_use]
    pub fn with_vanity_name(mut self, vanity_name: &'a str) -> Self {
        self.vanity_name = Some(vanity_name);
        self
    }

    /// 共有リンクを無効にする日時を設定する。
    #[must_use]
    pub fn with_unshared_at(mut self, unshared_at: OffsetDateTime) -> Self {
        self.unshared_at = Some(unshared_at);
        self
    }

    /// 共有リンクから項目をダウンロードできるかどうかを設定する。
    #[must_use]
    pub fn with_can_download(mut self, can_download: bool) -> Self {
        self.can_download = Some(can_download);
        self
    }

    /// ワンタイムパスコードの設定を設定する。
    #[must_use]
    pub fn with_one_time_passcode(mut self, one_time_passcode: OneTimePasscode<'a>) -> Self {
        self.one_time_passcode = Some(one_time_passcode);
        self
    }
}

/// 共有リンクの設定のリクエストボディ
///
/// パスワードを含むため、`Debug`を実装しない。
#[derive(Serialize)]
pub(crate) struct SharedLinkBody<'a> {
    /// 共有リンクにアクセスできるユーザーの範囲
    #[serde(skip_serializing_if = "Option::is_none")]
    access: Option<&'static str>,

    /// 共有リンクのパスワード
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<NullableBody<&'a str>>,

    /// バニティ名
    #[serde(skip_serializing_if = "Option::is_none")]
    vanity_name: Option<&'a str>,

    /// 共有リンクを無効にする日時
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    unshared_at: Option<OffsetDateTime>,

    /// 共有リンクの権限
    #[serde(skip_serializing_if = "Option::is_none")]
    permissions: Option<SharedLinkPermissionsBody>,

    /// ワンタイムパスコードの設定
    #[serde(skip_serializing_if = "Option::is_none")]
    one_time_passcode: Option<OneTimePasscodeBody<'a>>,
}

/// 共有リンクの権限のリクエストボディ
#[derive(Debug, Serialize)]
struct SharedLinkPermissionsBody {
    /// 共有リンクから項目をダウンロードできるかどうか
    can_download: bool,
}

/// ワンタイムパスコードの設定のリクエストボディ
#[derive(Debug, Serialize)]
struct OneTimePasscodeBody<'a> {
    /// ワンタイムパスコードのモード
    mode: &'static str,

    /// 許可リスト
    #[serde(skip_serializing_if = "Option::is_none")]
    allowlist: Option<Vec<OtpAllowlistEntryBody<'a>>>,
}

/// ワンタイムパスコードの許可リストの項目のリクエストボディ
#[derive(Debug, Serialize)]
struct OtpAllowlistEntryBody<'a> {
    /// 項目の種類
    #[serde(rename = "type")]
    entry_type: &'static str,

    /// メールアドレスまたはドメイン
    value: &'a str,
}

impl<'a> SharedLinkBody<'a> {
    /// 共有リンクの設定からボディを作成する。
    pub(crate) fn new(settings: &SharedLinkSettings<'a>) -> Self {
        Self {
            access: settings.access.map(SharedLinkAccess::as_str),
            password: settings
                .password
                .map(|password| password.to_body(|password| *password)),
            vanity_name: settings.vanity_name,
            unshared_at: settings.unshared_at,
            permissions: settings
                .can_download
                .map(|can_download| SharedLinkPermissionsBody { can_download }),
            one_time_passcode: settings.one_time_passcode.map(OneTimePasscodeBody::new),
        }
    }
}

impl<'a> OneTimePasscodeBody<'a> {
    /// ワンタイムパスコードの設定からボディを作成する。
    fn new(otp: OneTimePasscode<'a>) -> Self {
        match otp {
            OneTimePasscode::Disabled => Self {
                mode: "none",
                allowlist: None,
            },
            OneTimePasscode::Open => Self {
                mode: "open",
                allowlist: None,
            },
            OneTimePasscode::Allowlist(entries) => Self {
                mode: "allowlist",
                allowlist: Some(
                    entries
                        .iter()
                        .map(|entry| match *entry {
                            OtpAllowlistEntry::Email(value) => OtpAllowlistEntryBody {
                                entry_type: "email",
                                value,
                            },
                            OtpAllowlistEntry::Domain(value) => OtpAllowlistEntryBody {
                                entry_type: "domain",
                                value,
                            },
                        })
                        .collect(),
                ),
            },
        }
    }
}

/// コレクションのリクエストボディ
#[derive(Debug, Serialize)]
pub(crate) struct CollectionBody<'a> {
    /// コレクションのID
    id: &'a str,
}

/// コレクションのIDからリクエストボディを作成する。
pub(crate) fn collections_body<'a>(ids: Option<&[&'a str]>) -> Option<Vec<CollectionBody<'a>>> {
    ids.map(|ids| ids.iter().map(|&id| CollectionBody { id }).collect())
}

/// 更新する名前、説明およびタグを検証する。
///
/// # Errors
///
/// 名前をBOXが拒否する場合、または説明が[`MAX_DESCRIPTION_CHARS`](crate::MAX_DESCRIPTION_CHARS)文字より長い場合は、
/// [`BoxClientError::InvalidRequestBody`]を返す。
/// タグの数が1以上[`MAX_TAGS`]以下でない場合は、[`BoxClientError::InvalidArgument`]を返す。
pub(crate) fn validate_common_attributes(
    name: Option<&str>,
    description: Option<&str>,
    tags: Option<&[&str]>,
) -> BoxClientResult<()> {
    if let Some(name) = name {
        RenameBody::new(name)?;
    }
    if let Some(description) = description {
        SetDescriptionBody::new(description)?;
    }
    if let Some(tags) = tags
        && !(1..=MAX_TAGS).contains(&tags.len())
    {
        return Err(BoxClientError::InvalidArgument(
            "tags must contain between 1 and 100 tags",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod test_shared_link_body {
    use serde_json::json;
    use time::macros::datetime;

    use super::*;

    /// すべての設定を、BOXが受け付ける形式でシリアライズすることを確認
    #[test]
    fn test_serialize_all_settings() -> anyhow::Result<()> {
        let allowlist = [
            OtpAllowlistEntry::Email("user@example.com"),
            OtpAllowlistEntry::Domain("example.com"),
        ];
        let settings = SharedLinkSettings::default()
            .with_access(SharedLinkAccess::Open)
            .with_password("Secret-Passw0rd")
            .with_vanity_name("my-vanity-name")
            .with_unshared_at(datetime!(2026-12-31 23:59:59 UTC))
            .with_can_download(false)
            .with_one_time_passcode(OneTimePasscode::Allowlist(&allowlist));

        let value = serde_json::to_value(SharedLinkBody::new(&settings))?;

        assert_eq!(
            value,
            json!({
                "access": "open",
                "password": "Secret-Passw0rd",
                "vanity_name": "my-vanity-name",
                "unshared_at": "2026-12-31T23:59:59Z",
                "permissions": {"can_download": false},
                "one_time_passcode": {
                    "mode": "allowlist",
                    "allowlist": [
                        {"type": "email", "value": "user@example.com"},
                        {"type": "domain", "value": "example.com"},
                    ],
                },
            })
        );

        Ok(())
    }

    /// 設定しない属性を送信せず、削除するパスワードをnullで送信することを確認
    #[test]
    fn test_serialize_password_removal() -> anyhow::Result<()> {
        let settings = SharedLinkSettings::default()
            .with_password_removed()
            .with_one_time_passcode(OneTimePasscode::Disabled);

        let value = serde_json::to_value(SharedLinkBody::new(&settings))?;

        assert_eq!(
            value,
            json!({"password": null, "one_time_passcode": {"mode": "none"}})
        );

        Ok(())
    }

    /// デバッグ出力にパスワードを含めないことを確認
    #[test]
    fn test_debug_redacts_password() {
        let settings = SharedLinkSettings::default().with_password("Secret-Passw0rd");

        let debug = format!("{settings:?}");

        assert!(!debug.contains("Secret-Passw0rd"), "debug: {debug}");
        assert!(debug.contains("[REDACTED]"), "debug: {debug}");
    }
}

#[cfg(test)]
mod test_validate_common_attributes {
    use rstest::rstest;

    use super::*;

    /// タグの数が1以上100以下の場合だけ受け付けることを確認
    #[rstest]
    #[case::empty(0, false)]
    #[case::one(1, true)]
    #[case::max(MAX_TAGS, true)]
    #[case::too_many(MAX_TAGS + 1, false)]
    fn test_tags_count(#[case] count: usize, #[case] valid: bool) {
        let tags = vec!["tag"; count];

        let result = validate_common_attributes(None, None, Some(&tags));

        assert_eq!(result.is_ok(), valid, "result: {result:?}");
    }

    /// BOXが拒否する名前と長すぎる説明を拒否することを確認
    #[test]
    fn test_rejects_invalid_name_and_description() {
        let description = "a".repeat(crate::MAX_DESCRIPTION_CHARS + 1);

        assert!(validate_common_attributes(Some("a/b"), None, None).is_err());
        assert!(validate_common_attributes(None, Some(&description), None).is_err());
    }
}
