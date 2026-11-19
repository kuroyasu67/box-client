//! ユーザーに関するAPI

use serde::Deserialize;
use time::OffsetDateTime;

use crate::{BoxClient, BoxClientResult, UserId, request::ApiRequest};

/// BOXのユーザー
///
/// BOXのAPI仕様（OpenAPI）は、`id`と`type`だけを必須とし、表示名とメールアドレスを省略可能としている。
/// BOXが省略しても取得を失敗させないように、`id`と`type`以外の属性を`Option`で受け取る。
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
