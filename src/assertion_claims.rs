use serde::Serialize;

/// JWTアサーションのクレーム
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AssertionClaims<'a> {
    /// 発行者。クライアントIDを指定する。
    pub iss: &'a str,

    /// 主体。エンタープライズIDまたはユーザーIDを指定する。
    pub sub: &'a str,

    /// 主体の種類。`enterprise`または`user`を指定する。
    pub box_sub_type: &'a str,

    /// 受信者。トークンエンドポイントのURLを指定する。
    pub aud: &'a str,

    /// JWTアサーションの一意な識別子
    pub jti: &'a str,

    /// Unixエポックからの秒数で表した有効期限
    pub exp: u64,
}
