use axum::http::{header, HeaderMap, HeaderValue};
use cookie::{Cookie, SameSite};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use thiserror::Error;

pub const SESSION_SECONDS: i64 = 12 * 60 * 60;
pub const ADMIN_COOKIE: &str = "lc_admin";
pub const CSRF_COOKIE: &str = "lc_csrf";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("会话密钥非法")]
    InvalidKey,
    #[error("会话格式非法")]
    Malformed,
    #[error("会话签名无效")]
    BadSignature,
    #[error("会话已过期")]
    Expired,
    #[error("会话不属于该比赛")]
    ContestMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudentSession {
    pub contest_id: String,
    pub student_id: String,
    pub exp_ms: i64,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminSession {
    pub exp_ms: i64,
    pub token: String,
}

#[derive(Clone)]
pub struct SessionSigner {
    key: [u8; 32],
}

impl SessionSigner {
    pub fn from_hex(value: &str) -> Result<Self, AuthError> {
        let bytes = hex::decode(value).map_err(|_| AuthError::InvalidKey)?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| AuthError::InvalidKey)?;
        Ok(Self { key })
    }

    pub fn key_bytes(&self) -> [u8; 32] {
        self.key
    }

    pub fn sign_student(&self, contest_id: &str, student_id: &str, now_ms: i64) -> String {
        let exp_ms = now_ms + SESSION_SECONDS * 1000;
        let payload = format!("v1|student|{contest_id}|{student_id}|{exp_ms}");
        let mac = hex::encode(self.mac(payload.as_bytes()));
        format!(
            "v1.{}.{}.{}.{}",
            hex::encode(contest_id.as_bytes()),
            hex::encode(student_id.as_bytes()),
            exp_ms,
            mac
        )
    }

    pub fn verify_student(
        &self,
        token: &str,
        contest_id: &str,
        now_ms: i64,
    ) -> Result<StudentSession, AuthError> {
        let parts = token.split('.').collect::<Vec<_>>();
        if parts.len() != 5 || parts[0] != "v1" {
            return Err(AuthError::Malformed);
        }
        let contest = decode_hex_string(parts[1])?;
        let student_id = decode_hex_string(parts[2])?;
        let exp_ms = parts[3].parse::<i64>().map_err(|_| AuthError::Malformed)?;
        if contest != contest_id {
            return Err(AuthError::ContestMismatch);
        }
        if exp_ms <= now_ms {
            return Err(AuthError::Expired);
        }
        let payload = format!("v1|student|{contest}|{student_id}|{exp_ms}");
        if !self.verify_mac(payload.as_bytes(), parts[4]) {
            return Err(AuthError::BadSignature);
        }
        Ok(StudentSession {
            contest_id: contest,
            student_id,
            exp_ms,
            token: token.to_string(),
        })
    }

    pub fn sign_admin(&self, now_ms: i64) -> String {
        let exp_ms = now_ms + SESSION_SECONDS * 1000;
        let payload = format!("v1|admin|{exp_ms}");
        let mac = hex::encode(self.mac(payload.as_bytes()));
        format!("v1.{exp_ms}.{mac}")
    }

    pub fn verify_admin(&self, token: &str, now_ms: i64) -> Result<AdminSession, AuthError> {
        let parts = token.split('.').collect::<Vec<_>>();
        if parts.len() != 3 || parts[0] != "v1" {
            return Err(AuthError::Malformed);
        }
        let exp_ms = parts[1].parse::<i64>().map_err(|_| AuthError::Malformed)?;
        if exp_ms <= now_ms {
            return Err(AuthError::Expired);
        }
        let payload = format!("v1|admin|{exp_ms}");
        if !self.verify_mac(payload.as_bytes(), parts[2]) {
            return Err(AuthError::BadSignature);
        }
        Ok(AdminSession {
            exp_ms,
            token: token.to_string(),
        })
    }

    fn mac(&self, data: &[u8]) -> [u8; 32] {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts 32-byte keys");
        mac.update(data);
        mac.finalize().into_bytes().into()
    }

    fn verify_mac(&self, data: &[u8], expected_hex: &str) -> bool {
        let Ok(expected) = hex::decode(expected_hex) else {
            return false;
        };
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts 32-byte keys");
        mac.update(data);
        mac.verify_slice(&expected).is_ok()
    }
}

fn decode_hex_string(value: &str) -> Result<String, AuthError> {
    let bytes = hex::decode(value).map_err(|_| AuthError::Malformed)?;
    String::from_utf8(bytes).map_err(|_| AuthError::Malformed)
}

pub fn student_cookie_name(contest_id: &str) -> String {
    format!("lc_s_{contest_id}")
}

pub fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    for header in headers.get_all(header::COOKIE) {
        let Ok(value) = header.to_str() else {
            continue;
        };
        for cookie in Cookie::split_parse(value.to_owned()) {
            let Ok(cookie) = cookie else {
                continue;
            };
            if cookie.name() == name {
                return Some(cookie.value().to_string());
            }
        }
    }
    None
}

pub fn build_cookie(name: &str, value: &str, max_age_seconds: i64) -> HeaderValue {
    let cookie = Cookie::build((name.to_string(), value.to_string()))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(cookie::time::Duration::seconds(max_age_seconds))
        .build();
    HeaderValue::from_str(&cookie.to_string()).expect("cookie header is valid")
}

pub fn clear_cookie(name: &str) -> HeaderValue {
    build_cookie(name, "", 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn signer() -> SessionSigner {
        SessionSigner::from_hex(&"11".repeat(32)).unwrap()
    }

    #[test]
    fn session_round_trip_and_tamper_detection() {
        let signer = signer();
        let token = signer.sign_student("demo", "2026001", 1000);
        let session = signer.verify_student(&token, "demo", 2000).unwrap();
        assert_eq!(session.student_id, "2026001");
        assert_eq!(session.contest_id, "demo");
        assert_eq!(session.token, token);

        let mut tampered = token.clone();
        tampered.push('0');
        assert!(signer.verify_student(&tampered, "demo", 2000).is_err());
        assert!(matches!(
            signer.verify_student(&token, "other", 2000),
            Err(AuthError::ContestMismatch)
        ));
    }

    #[test]
    fn session_expires() {
        let signer = signer();
        let token = signer.sign_student("demo", "1", 1000);
        assert!(matches!(
            signer.verify_student(&token, "demo", 1000 + SESSION_SECONDS * 1000 + 1),
            Err(AuthError::Expired)
        ));
    }

    #[test]
    fn admin_session_round_trip_and_expiry() {
        let signer = signer();
        let token = signer.sign_admin(1000);
        assert!(signer.verify_admin(&token, 2000).is_ok());
        assert!(matches!(
            signer.verify_admin(&token, 1000 + SESSION_SECONDS * 1000 + 1),
            Err(AuthError::Expired)
        ));
        assert!(matches!(
            signer.verify_admin("v1.bad", 0),
            Err(AuthError::Malformed)
        ));
    }

    #[test]
    fn cookie_round_trip_and_clear() {
        let headers = HeaderMap::new();
        assert!(read_cookie(&headers, "missing").is_none());

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; lc_s_demo=abc; b=2"),
        );
        assert_eq!(read_cookie(&headers, "lc_s_demo").as_deref(), Some("abc"));

        let header = build_cookie("lc_s_demo", "abc", 3600);
        let text = header.to_str().unwrap();
        assert!(text.contains("HttpOnly"));
        assert!(text.contains("SameSite=Lax"));
        assert!(text.contains("Path=/"));
        assert!(text.contains("Max-Age=3600"));
        assert!(clear_cookie("lc_s_demo")
            .to_str()
            .unwrap()
            .contains("Max-Age=0"));
    }

    #[test]
    fn rejects_bad_keys_and_tokens() {
        assert!(matches!(
            SessionSigner::from_hex("00"),
            Err(AuthError::InvalidKey)
        ));
        assert!(matches!(
            signer().verify_student("bad", "demo", 0),
            Err(AuthError::Malformed)
        ));
    }
}
