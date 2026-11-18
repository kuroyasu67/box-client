use serde::{Deserialize, Serialize};

/// BOXの項目のIDを表す型を定義する。
///
/// `$item`には、ドキュメントに使用する項目の種類を指定する。
macro_rules! box_item_struct {
    ($name: ident, $item: literal) => {
        #[doc = concat!(
            $item,
            "のID\n\n",
            "BOXはIDを数字の文字列で返すが、このクライアントは値を検証せずに保持する。\n",
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
box_item_struct!(FolderId, "フォルダ");
box_item_struct!(FileId, "ファイル");
box_item_struct!(WebLinkId, "Webリンク");

impl FolderId {
    /// ルートフォルダのIDを返す。
    ///
    /// BOXでは、ルートフォルダ（「すべてのファイル」）のIDは常に`0`である。
    #[must_use]
    pub fn root() -> Self {
        FolderId::new("0")
    }

    /// ルートフォルダのIDであるかを返す。
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0 == "0"
    }
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

    /// このクライアントが知らない種類の項目
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

    /// このクライアントが知らない状態
    ///
    /// BOXが状態を追加しても、項目の情報の取得を失敗させないために受け入れる。
    #[serde(other)]
    Unknown,
}

/// 親フォルダ
#[derive(Debug, Clone, Serialize)]
pub struct ParentFolder {
    /// 親フォルダのID
    pub id: FolderId,
}

/// ファイル名またはフォルダ名に使用できない文字が含まれていないか検証する。
///
/// BOXは、ファイル名とフォルダ名のどちらでも、ASCIIの非印字文字、`/`または`\`を含む名前、
/// 末尾が空白の名前、`.`および`..`を拒否する。
/// `garde`の`custom`バリデーターとして、`#[garde(custom(validate_item_name))]`のように使用する。
///
/// BOXは名前の長さも1文字以上255文字以下に制限する。長さは文字数で数えるため、
/// このバリデーターでは検証せず、`#[garde(length(chars, min = 1, max = 255))]`と組み合わせて検証する。
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
