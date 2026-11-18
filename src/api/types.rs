use garde::Validate;
use serde::{Deserialize, Serialize};

use crate::{
    BoxClientError, BoxClientResult, ValidationError, request::ApiRequest, retry::Idempotency,
};

/// 項目名の最大文字数
///
/// BOXは、ファイル、フォルダおよびWebリンクのどの名前も、1文字以上255文字以下に制限する。
/// BOXは、名前の長さを文字数で数える。
pub const MAX_NAME_CHARS: usize = 255;

/// リクエストボディを検証し、BOXが受け付ける場合はそのまま返す。
///
/// # Errors
///
/// BOXが受け付けない値が含まれる場合は、[`BoxClientError::InvalidRequestBody`]を返す。
pub(crate) fn validated<T: Validate<Context = ()>>(body: T) -> BoxClientResult<T> {
    body.validate()
        .map_err(|report| BoxClientError::InvalidRequestBody(ValidationError::new(report)))?;
    Ok(body)
}

/// ファイル、フォルダまたはWebリンクの属性を更新するリクエスト（`PUT /{collection}/{id}`）を作成する。
///
/// 名前の変更、説明の設定、移動およびWebリンクのURLの変更で共有する。
/// `etag`を指定した場合は、`If-Match`ヘッダーで送信する。
///
/// `ETag`を指定した場合は、BOXが更新した後に再送すると、`ETag`が変わっているためBOXは412を返す。
/// その結果、呼び出し元が更新の結果を誤って判断するおそれがあるため、このリクエストは、HTTPメソッドがPUTであっても冪等でないリクエストとして送信する。
///
/// # Errors
///
/// ボディをJSONにシリアライズできない場合は、[`BoxClientError::SerializeBody`]を返す。
/// `etag`に使用できない文字が含まれる場合は、[`BoxClientError::InvalidHeaderValue`]を返す。
pub(crate) fn update_item_request<'a, B: Serialize + ?Sized>(
    collection: &'static str,
    id: &'a str,
    body: &B,
    etag: Option<&str>,
) -> BoxClientResult<ApiRequest<'a>> {
    ApiRequest::put([collection, id])
        .idempotency(Idempotency::NonIdempotent)
        .json(body)?
        .if_match(etag)
}

/// BOXの項目のIDを表す型を定義する。
///
/// `$item`には、ドキュメントに使用する項目の種類を指定する。
macro_rules! box_item_struct {
    ($name: ident, $item: literal) => {
        #[doc = concat!(
            $item,
            "のID\n\n",
            "BOXはIDを数字の文字列で返すが、BOXクライアントは値を検証せずに保持する。\n",
            "空文字列などの不正なIDは、APIを呼び出したときに",
            "[`BoxClientError::InvalidPathSegment`](crate::BoxClientError::InvalidPathSegment)になる。",
        )]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
        pub struct $name(#[doc = "IDの文字列"] pub String);

        impl $name {
            #[doc = concat!("文字列から", $item, "のIDを作成する。")]
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

box_item_struct!(UserId, "ユーザー");
box_item_struct!(FileVersionId, "ファイルバージョン");
box_item_struct!(FolderId, "フォルダ");
box_item_struct!(FileId, "ファイル");
box_item_struct!(WebLinkId, "Webリンク");

/// ルートフォルダのID
///
/// BOXでは、ルートフォルダ（「すべてのファイル」）のIDは常に`0`である。
const ROOT_FOLDER_ID: &str = "0";

impl FolderId {
    /// ルートフォルダのIDを返す。
    ///
    /// BOXでは、ルートフォルダ（「すべてのファイル」）のIDは常に`0`である。
    #[must_use]
    pub fn root() -> Self {
        FolderId::new(ROOT_FOLDER_ID)
    }

    /// ルートフォルダのIDであるかを返す。
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0 == ROOT_FOLDER_ID
    }
}

/// ルートフォルダを対象にできないAPIで、フォルダIDがルートフォルダでないことを検証する。
///
/// BOXはルートフォルダのコピー、移動、削除などを受け付けないため、リクエストを送信する前に検出する。
///
/// # Arguments
///
/// - `folder_id`: 検証するフォルダID
/// - `message`: ルートフォルダの場合に返すエラーのメッセージ
///
/// # Errors
///
/// `folder_id`がルートフォルダの場合は、[`BoxClientError::InvalidArgument`]を返す。
pub(crate) fn ensure_not_root(folder_id: &FolderId, message: &'static str) -> BoxClientResult<()> {
    if folder_id.is_root() {
        return Err(BoxClientError::InvalidArgument(message));
    }
    Ok(())
}

/// フォルダ内の項目
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum BoxItem {
    /// ファイル
    File(FileItem),

    /// フォルダ
    Folder(FolderItem),

    /// Webリンク
    WebLink(WebLinkItem),

    /// BOXクライアントが知らない種類の項目
    ///
    /// BOXが項目の種類を追加しても、ページ全体の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

/// フォルダ内のファイル
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct FileItem {
    /// ファイルID
    pub id: FileId,

    /// ファイル名
    pub name: String,

    /// ファイルのバージョンを識別する`ETag`
    pub etag: Option<String>,
}

/// フォルダ内のフォルダ
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct FolderItem {
    /// フォルダID
    pub id: FolderId,

    /// フォルダ名
    pub name: String,

    /// フォルダのバージョンを識別する`ETag`
    pub etag: Option<String>,
}

/// フォルダ内のWebリンク
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct WebLinkItem {
    /// WebリンクID
    pub id: WebLinkId,

    /// Webリンク名
    pub name: String,

    /// Webリンクのバージョンを識別する`ETag`
    pub etag: Option<String>,
}

/// ルートフォルダから項目の親フォルダまでのパス
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct PathCollection {
    /// パスに含まれるフォルダの数
    pub total_count: u64,

    /// ルートフォルダから順に並べた、パスに含まれるフォルダ
    pub entries: Vec<FolderItem>,
}

/// 項目の状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ItemStatus {
    /// 有効
    Active,

    /// ゴミ箱に移動済み
    Trashed,

    /// 削除済み
    Deleted,

    /// BOXクライアントが知らない状態
    ///
    /// BOXが状態を追加しても、項目の情報の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

/// リクエストボディで指定する親フォルダ
///
/// BOXは、作成、コピー、アップロードおよび復元で、親フォルダを`{"id": "..."}`の形式で受け付けるため、
/// これらのリクエストボディで共有する。IDを複製しないように、IDを参照で保持する。
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct ParentFolder<'a> {
    /// 親フォルダのID
    id: &'a FolderId,
}

impl<'a> ParentFolder<'a> {
    /// 親フォルダのIDから作成する。
    pub(crate) fn new(id: &'a FolderId) -> Self {
        Self { id }
    }
}

/// ファイル、フォルダまたはWebリンクの名前に使用できない文字が含まれていないか検証する。
///
/// BOXは、ファイル名、フォルダ名およびWebリンク名のどれでも、ASCIIの非印字文字、`/`または`\`を含む名前、
/// 末尾が空白の名前、`.`および`..`を拒否する。
/// `garde`の`custom`バリデーターとして、`#[garde(custom(validate_item_name))]`のように使用する。
///
/// BOXは名前の長さも1文字以上255文字以下に制限する。長さは文字数で数えるため、
/// このバリデーターでは検証せず、`#[garde(length(chars, min = 1, max = MAX_NAME_CHARS))]`と組み合わせて検証する。
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "gardeのcustomバリデーターは、コンテキストを参照で受け取るシグネチャを要求するため"
)]
pub(crate) fn validate_item_name(name: &str, _context: &()) -> garde::Result {
    if name == "." || name == ".." {
        return Err(garde::Error::new("must not be `.` or `..`"));
    }
    if let Some(c) = name
        .chars()
        .find(|c| c.is_ascii_control() || matches!(c, '/' | '\\'))
    {
        return Err(garde::Error::new(format!("must not contain {c:?}")));
    }
    if name.ends_with(' ') {
        return Err(garde::Error::new("must not end with a space"));
    }
    Ok(())
}

/// ファイル、フォルダまたはWebリンクの情報を更新するAPIのオプション
///
/// BOXは、ファイル、フォルダおよびWebリンクの情報の更新で、同じ形式の`If-Match`ヘッダーを受け付けるため、
/// 名前の変更、説明の設定および移動などのメソッドで共有する。
///
/// # Examples
///
/// ```
/// use box_client::UpdateItemOptions;
///
/// let options = UpdateItemOptions::default().with_etag("1");
/// assert_eq!(options.etag, Some("1"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct UpdateItemOptions<'a> {
    /// 最後に取得した項目の`ETag`
    ///
    /// 指定した場合は、`If-Match`ヘッダーで送信する。項目が`ETag`の取得後に変更されていると、
    /// BOXは項目を更新せずに412を返す。
    pub etag: Option<&'a str>,
}

impl<'a> UpdateItemOptions<'a> {
    /// 最後に取得した項目の`ETag`を設定する。
    #[must_use]
    pub fn with_etag(mut self, etag: &'a str) -> Self {
        self.etag = Some(etag);
        self
    }
}

/// ファイルまたはファイルのバージョンを削除するAPIのオプション
///
/// BOXは、ファイルの削除とファイルのバージョンの削除で、同じ形式の`If-Match`ヘッダーを受け付けるため、
/// [`BoxClient::delete_file`](crate::BoxClient::delete_file)および
/// [`BoxClient::delete_file_version`](crate::BoxClient::delete_file_version)で共有する。
/// フォルダの削除は、空でないフォルダを削除するかどうかも指定するため、
/// [`DeleteFolderOptions`](crate::DeleteFolderOptions)を使用する。
///
/// # Examples
///
/// ```
/// use box_client::DeleteItemOptions;
///
/// let options = DeleteItemOptions::default().with_etag("1");
/// assert_eq!(options.etag, Some("1"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeleteItemOptions<'a> {
    /// 最後に取得したファイルの`ETag`
    ///
    /// 指定した場合は、`If-Match`ヘッダーで送信する。ファイルが`ETag`の取得後に変更されていると、
    /// BOXはファイルまたはバージョンを削除せずに412を返す。
    pub etag: Option<&'a str>,
}

impl<'a> DeleteItemOptions<'a> {
    /// 最後に取得したファイルの`ETag`を設定する。
    #[must_use]
    pub fn with_etag(mut self, etag: &'a str) -> Self {
        self.etag = Some(etag);
        self
    }
}

/// ファイル、フォルダまたはWebリンクの名前変更リクエストのボディ
#[derive(Debug, Clone, Serialize, Validate)]
pub(crate) struct RenameBody<'a> {
    /// 新しい名前
    ///
    /// BOXは、名前の長さを文字数で数える。
    #[garde(length(chars, min = 1, max = MAX_NAME_CHARS), custom(validate_item_name))]
    name: &'a str,
}

impl<'a> RenameBody<'a> {
    /// ボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// 名前をBOXが拒否する場合は、[`BoxClientError::InvalidRequestBody`]を返す。
    pub(crate) fn new(name: &'a str) -> BoxClientResult<Self> {
        validated(Self { name })
    }
}

/// ファイル、フォルダまたはWebリンクの説明の最大文字数
///
/// BOXは、ファイル、フォルダおよびWebリンクのどの説明も、256文字以下に制限する。
pub const MAX_DESCRIPTION_CHARS: usize = 256;

/// ファイル、フォルダまたはWebリンクの説明設定リクエストのボディ
///
/// BOXは、ファイル、フォルダおよびWebリンクの説明を同じ形式で受け付けるため、
/// [`BoxClient::set_file_description`](crate::BoxClient::set_file_description)、
/// [`BoxClient::set_folder_description`](crate::BoxClient::set_folder_description)および
/// [`BoxClient::set_web_link_description`](crate::BoxClient::set_web_link_description)で共有する。
#[derive(Debug, Clone, Serialize, Validate)]
pub(crate) struct SetDescriptionBody<'a> {
    /// 説明
    ///
    /// BOXは説明にnullを受け付けないため、説明を削除する場合は空文字列を送信する。
    #[garde(length(chars, max = MAX_DESCRIPTION_CHARS))]
    description: &'a str,
}

impl<'a> SetDescriptionBody<'a> {
    /// ボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// 説明が[`MAX_DESCRIPTION_CHARS`]文字より長い場合は、[`BoxClientError::InvalidRequestBody`]を返す。
    pub(crate) fn new(description: &'a str) -> BoxClientResult<Self> {
        validated(Self { description })
    }
}

/// ファイル、フォルダまたはWebリンクの移動先
///
/// BOXは、ファイル、フォルダおよびWebリンクの移動先を同じ形式で受け付けるため、
/// [`BoxClient::move_file`](crate::BoxClient::move_file)、
/// [`BoxClient::move_folder`](crate::BoxClient::move_folder)および
/// [`BoxClient::move_web_link`](crate::BoxClient::move_web_link)で共有する。
///
/// # Examples
///
/// ```
/// use box_client::{FolderId, MoveDestination, UserId};
///
/// // IDが`456`のフォルダへ移動する。
/// let to_folder = MoveDestination::Folder(FolderId::new("456"));
/// // 認証中のユーザーのルートフォルダへ移動する。
/// let to_root = MoveDestination::Folder(FolderId::root());
/// // IDが`789`のユーザーのルートフォルダへ移動する。
/// let to_user = MoveDestination::User(UserId::new("789"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveDestination {
    /// IDで指定したフォルダ
    ///
    /// 認証中のユーザーのルートフォルダへ移動する場合は、[`FolderId::root`]を指定する。
    Folder(FolderId),

    /// IDで指定したユーザーのルートフォルダ
    ///
    /// BOXは、ユーザーIDを指定した場合に、ルートフォルダ以外への移動を受け付けない。
    /// したがって、ユーザーのルートフォルダだけを移動先にできる。
    User(UserId),
}

/// ファイル、フォルダまたはWebリンクの移動リクエストのボディ
///
/// BOXは、フォルダの移動先の親フォルダと他の属性を同時に指定すると、403（`operation_limit_exceeded`）を返す。
/// したがって、移動先の親フォルダだけを送信する。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct MoveBody<'a> {
    /// 移動先の親フォルダ
    parent: MoveParent<'a>,
}

/// 移動リクエストのボディの、移動先の親フォルダ
#[derive(Debug, Clone, Serialize)]
struct MoveParent<'a> {
    /// 移動先の親フォルダのID
    ///
    /// ユーザーのルートフォルダへ移動する場合は、ルートフォルダのIDを指定する。
    /// IDを複製しないように、IDを参照で保持する。
    id: &'a str,

    /// 移動先のルートフォルダを所有するユーザーのID
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<&'a UserId>,
}

impl<'a> MoveBody<'a> {
    /// 移動先からボディを作成する。
    ///
    /// # Errors
    ///
    /// ユーザーIDが空文字列の場合は、[`BoxClientError::InvalidArgument`]を返す。
    pub(crate) fn new(destination: &'a MoveDestination) -> BoxClientResult<Self> {
        let parent = match destination {
            MoveDestination::Folder(folder_id) => MoveParent {
                id: folder_id.as_ref(),
                user_id: None,
            },
            MoveDestination::User(user_id) => {
                // フォルダIDと異なり、ユーザーIDはパスに含めないため、空文字列をクライアントが検出しないとBOXへ送信してしまう。
                // BOXが空のユーザーIDをどう扱うかは不明であり、認証中のユーザーのルートフォルダへ移動するおそれもあるため、送信しない。
                if user_id.as_ref().is_empty() {
                    return Err(BoxClientError::InvalidArgument("user_id must not be empty"));
                }
                MoveParent {
                    id: ROOT_FOLDER_ID,
                    user_id: Some(user_id),
                }
            }
        };
        Ok(Self { parent })
    }
}

/// 省略可能なファイル、フォルダまたはWebリンクの名前に、使用できない文字が含まれていないか検証する。
///
/// `None`の場合は検証しない。`garde`の`custom`バリデーターは、フィールドの型の参照を受け取るため、
/// `Option<&str>`のフィールドには[`validate_item_name`]を直接適用できない。
#[expect(
    clippy::ref_option_ref,
    clippy::trivially_copy_pass_by_ref,
    reason = "gardeのcustomバリデーターは、フィールドとコンテキストを参照で受け取るシグネチャを要求するため"
)]
pub(crate) fn validate_optional_item_name(name: &Option<&str>, context: &()) -> garde::Result {
    name.map_or(Ok(()), |name| validate_item_name(name, context))
}

#[cfg(test)]
mod test_validate_item_name {
    use rstest::rstest;

    use super::*;

    /// BOXが受け付ける名前を受け付けることを確認
    #[rstest]
    #[case::ascii("report.pdf")]
    #[case::japanese("報告書.pdf")]
    #[case::inner_space("my report.pdf")]
    #[case::leading_space(" report.pdf")]
    #[case::dots("...")]
    #[case::dot_prefix(".gitignore")]
    #[case::no_extension("README")]
    fn test_valid(#[case] name: &str) {
        let result = validate_item_name(name, &());

        assert!(result.is_ok(), "result: {result:?}");
    }

    /// BOXが拒否する名前を拒否することを確認
    #[rstest]
    #[case::dot(".")]
    #[case::dot_dot("..")]
    #[case::slash("a/b")]
    #[case::backslash("a\\b")]
    #[case::newline("a\nb")]
    #[case::tab("a\tb")]
    #[case::nul("a\0b")]
    #[case::del("a\u{7f}b")]
    #[case::trailing_space("report.pdf ")]
    fn test_invalid(#[case] name: &str) {
        let result = validate_item_name(name, &());

        assert!(result.is_err(), "result: {result:?}");
    }
}

#[cfg(test)]
mod test_folder_id_is_root {
    use rstest::rstest;

    use super::*;

    /// IDが0の場合だけルートフォルダと判定することを確認
    #[rstest]
    #[case::root("0", true)]
    #[case::other("1", false)]
    #[case::leading_zero("00", false)]
    #[case::empty("", false)]
    fn test_is_root(#[case] id: &str, #[case] expected: bool) {
        assert_eq!(FolderId::new(id).is_root(), expected);
    }
}

#[cfg(test)]
mod test_item_status {
    use rstest::rstest;
    use serde_json::json;

    use super::*;

    /// 項目の状態をデシリアライズし、知らない状態を受け入れることを確認
    #[rstest]
    #[case::active("active", ItemStatus::Active)]
    #[case::trashed("trashed", ItemStatus::Trashed)]
    #[case::deleted("deleted", ItemStatus::Deleted)]
    #[case::unknown("archived", ItemStatus::Unknown)]
    fn test_deserialize(#[case] value: &str, #[case] expected: ItemStatus) -> anyhow::Result<()> {
        let status: ItemStatus = serde_json::from_value(json!(value))?;

        assert_eq!(status, expected);

        Ok(())
    }
}
