//! ユーザーに関するAPI

use serde::Deserialize;

use crate::{BoxClient, BoxClientResult, request::ApiRequest};

/// Boxのユーザー
#[derive(Debug, Clone, Deserialize)]
pub struct User {
    /// ユーザーID
    pub id: String,

    /// 表示名
    pub name: String,

    /// ログインに使用するメールアドレス
    pub login: String,
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

    /// GET /users/meを呼び出し、ユーザー情報をデシリアライズすることを確認
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
        let client = create_client(&server, short_retry(0)?).await?;

        let user = client.me().await?;

        assert_eq!(user.id, "11446498");
        assert_eq!(user.name, "Service Account");
        assert_eq!(user.login, "AutomationUser_123@boxdevedition.com");

        Ok(())
    }
}
