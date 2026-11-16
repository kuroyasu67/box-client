use std::{fs::File, io::BufReader, path::Path};

use secrecy::SecretString;
use serde::Deserialize;

use crate::errors::{BoxClientError, BoxClientResult};

/// BOX設定
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxConfig {
    /// BOXアプリ設定
    pub box_app_settings: BoxAppSettings,

    /// エンタープライズID
    #[serde(rename = "enterpriseID")]
    pub enterprise_id: String,
}

/// BOXアプリ設定
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxAppSettings {
    /// クライアントID
    #[serde(rename = "clientID")]
    pub client_id: String,

    /// クライアントシークレット
    pub client_secret: SecretString,

    /// BOXアプリ認証情報
    pub app_auth: BoxAppAuth,
}

/// BOXアプリ認証情報
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxAppAuth {
    /// 公開鍵ID
    #[serde(rename = "publicKeyID")]
    pub public_key_id: String,

    /// 秘密鍵
    pub private_key: SecretString,

    /// 秘密鍵を復号するパスフレーズ
    pub passphrase: SecretString,
}

impl BoxConfig {
    /// BOX設定をファイルから読み込む。
    ///
    /// # Arguments
    ///
    /// - `path`: BOX設定ファイルのパス
    ///
    /// # Errors
    ///
    /// ファイルを開けない場合、またはBOX設定として解釈できない場合はエラーを返す。
    pub fn from_file<P: AsRef<Path>>(path: P) -> BoxClientResult<Self> {
        let file = File::open(path).map_err(BoxClientError::OpenConfigFile)?;
        let reader = BufReader::new(file);

        serde_json::from_reader(reader).map_err(BoxClientError::ParseConfig)
    }
}

#[cfg(test)]
mod test_box_config {
    use secrecy::ExposeSecret;

    use super::*;

    /// BOX設定ファイルの形式のJSONを読み込めることを確認
    #[test]
    fn test_deserialize_box_config_json() -> anyhow::Result<()> {
        let json = r#"{
            "boxAppSettings": {
                "clientID": "client-id",
                "clientSecret": "client-secret",
                "appAuth": {
                    "publicKeyID": "public-key-id",
                    "privateKey": "private-key",
                    "passphrase": "passphrase"
                }
            },
            "enterpriseID": "12345"
        }"#;

        let config: BoxConfig = serde_json::from_str(json)?;
        let settings = &config.box_app_settings;

        assert_eq!(settings.client_id, "client-id");
        assert_eq!(settings.client_secret.expose_secret(), "client-secret");
        assert_eq!(settings.app_auth.public_key_id, "public-key-id");
        assert_eq!(settings.app_auth.private_key.expose_secret(), "private-key");
        assert_eq!(settings.app_auth.passphrase.expose_secret(), "passphrase");
        assert_eq!(config.enterprise_id, "12345");

        Ok(())
    }

    /// BOX設定ファイルが存在しない場合は、ファイルのオープンエラーを返すことを確認
    #[test]
    fn test_from_file_not_found() {
        let result = BoxConfig::from_file("resources/tests/not_found.json");

        assert!(
            matches!(result, Err(BoxClientError::OpenConfigFile(_))),
            "result: {result:?}"
        );
    }
}
