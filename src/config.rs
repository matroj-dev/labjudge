use std::fs;
use std::path::{Path, PathBuf};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub port: u16,
    pub admin_password: String,
    pub judge_concurrency: usize,
    pub submit_cooldown_seconds: u64,
    pub max_source_bytes: usize,
    pub session_secret: String,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("配置读写失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("配置 JSON 非法: {0}")]
    Json(#[from] serde_json::Error),
    #[error("配置校验失败: {0}")]
    Invalid(String),
}

impl Default for Config {
    fn default() -> Self {
        Self::for_test()
    }
}

impl Config {
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        let password = generate_admin_password(&mut rng);
        let mut secret = [0u8; 32];
        rng.fill_bytes(&mut secret);

        Self {
            port: 8080,
            admin_password: password,
            judge_concurrency: 2,
            submit_cooldown_seconds: 5,
            max_source_bytes: 65_536,
            session_secret: hex::encode(secret),
        }
    }

    pub fn for_test() -> Self {
        Self {
            port: 8080,
            admin_password: "000000".to_string(),
            judge_concurrency: 2,
            submit_cooldown_seconds: 5,
            max_source_bytes: 65_536,
            session_secret: "00".repeat(32),
        }
    }

    pub fn load_or_create(root: &Path) -> Result<(Self, bool), ConfigError> {
        let path = config_path(root);
        if path.exists() {
            let text = fs::read_to_string(&path)?;
            let config: Self = serde_json::from_str(&text)?;
            config.validate()?;
            return Ok((config, false));
        }

        let config = Self::generate();
        config.validate()?;
        config.save(root)?;
        Ok((config, true))
    }

    /// 启动前校验：配置错了直接退出，不能带着非法参数开赛。
    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |reason: String| ConfigError::Invalid(reason);
        if self.port == 0 {
            return Err(invalid("port 必须在 1～65535 之间".to_string()));
        }
        if !(1..=64).contains(&self.judge_concurrency) {
            return Err(invalid("judgeConcurrency 必须在 1～64 之间".to_string()));
        }
        if self.submit_cooldown_seconds > 3600 {
            return Err(invalid("submitCooldownSeconds 不能超过 3600".to_string()));
        }
        if !(1024..=4 * 1024 * 1024).contains(&self.max_source_bytes) {
            return Err(invalid(
                "maxSourceBytes 必须在 1024～4194304 之间".to_string(),
            ));
        }
        if self.admin_password.trim().is_empty() {
            return Err(invalid("adminPassword 不能为空".to_string()));
        }
        if self.session_secret.len() != 64
            || !self
                .session_secret
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid(
                "sessionSecret 必须是 64 位十六进制字符串".to_string(),
            ));
        }
        Ok(())
    }

    pub fn save(&self, root: &Path) -> Result<(), ConfigError> {
        fs::create_dir_all(root)?;
        let path = config_path(root);

        let mut object = if path.exists() {
            let text = fs::read_to_string(&path)?;
            serde_json::from_str::<serde_json::Value>(&text)?
                .as_object()
                .cloned()
                .unwrap_or_default()
        } else {
            serde_json::Map::new()
        };

        let value = serde_json::to_value(self)?;
        if let Some(fields) = value.as_object() {
            for (key, value) in fields {
                object.insert(key.clone(), value.clone());
            }
        }

        let text = serde_json::to_string_pretty(&serde_json::Value::Object(object))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        if let Err(error) = fs::rename(&tmp, &path) {
            fs::remove_file(&path).ok();
            fs::rename(&tmp, &path).map_err(|_| error)?;
        }
        Ok(())
    }
}

/// 首启生成的管理口令：12 位、约 60 bit 熵。
///
/// 去掉了 I/O/0/1 这类易混字符，教师手抄不会看错；
/// 局域网内不做登录限流，靠长度让爆破不划算。
const ADMIN_PASSWORD_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const ADMIN_PASSWORD_LEN: usize = 12;

fn generate_admin_password<R: rand::Rng>(rng: &mut R) -> String {
    (0..ADMIN_PASSWORD_LEN)
        .map(|_| {
            let index = rng.gen_range(0..ADMIN_PASSWORD_CHARS.len());
            ADMIN_PASSWORD_CHARS[index] as char
        })
        .collect()
}

pub fn config_path(root: &Path) -> PathBuf {
    root.join("config.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_generates_password_and_secret() {
        let dir = tempfile::tempdir().unwrap();
        let (config, first_run) = Config::load_or_create(dir.path()).unwrap();

        assert!(first_run);
        assert_eq!(config.admin_password.len(), ADMIN_PASSWORD_LEN);
        assert!(config
            .admin_password
            .bytes()
            .all(|b| ADMIN_PASSWORD_CHARS.contains(&b)));
        assert_eq!(config.session_secret.len(), 64);
        assert!(config.session_secret.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(config.port, 8080);
        assert_eq!(config.judge_concurrency, 2);
        assert_eq!(config.submit_cooldown_seconds, 5);
        assert_eq!(config.max_source_bytes, 65_536);
    }

    #[test]
    fn second_load_keeps_password_and_secret() {
        let dir = tempfile::tempdir().unwrap();
        let (first, _) = Config::load_or_create(dir.path()).unwrap();
        let (second, first_run) = Config::load_or_create(dir.path()).unwrap();

        assert!(!first_run);
        assert_eq!(first, second);
    }

    #[test]
    fn save_preserves_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let (config, _) = Config::load_or_create(dir.path()).unwrap();
        let path = config_path(dir.path());
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("extra".to_string(), serde_json::json!(123));
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        config.save(dir.path()).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["extra"], serde_json::json!(123));
        assert_eq!(
            saved["adminPassword"],
            serde_json::json!(config.admin_password)
        );
    }

    #[test]
    fn invalid_values_are_rejected_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::for_test();
        config.judge_concurrency = 0;
        fs::write(
            config_path(dir.path()),
            serde_json::to_string(&config).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            Config::load_or_create(dir.path()),
            Err(ConfigError::Invalid(_))
        ));

        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::for_test();
        config.session_secret = "xyz".to_string();
        fs::write(
            config_path(dir.path()),
            serde_json::to_string(&config).unwrap(),
        )
        .unwrap();
        let error = Config::load_or_create(dir.path()).unwrap_err();
        assert!(error.to_string().contains("sessionSecret"));
    }

    #[test]
    fn invalid_json_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(config_path(dir.path()), "{not json").unwrap();
        assert!(matches!(
            Config::load_or_create(dir.path()),
            Err(ConfigError::Json(_))
        ));
    }
}
