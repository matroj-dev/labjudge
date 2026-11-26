use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

use super::session::AuthError;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct Csrf {
    key: [u8; 32],
}

impl Csrf {
    pub fn from_hex(value: &str) -> Result<Self, AuthError> {
        let bytes = hex::decode(value).map_err(|_| AuthError::InvalidKey)?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| AuthError::InvalidKey)?;
        Ok(Self { key })
    }

    pub fn from_key(key: [u8; 32]) -> Self {
        Self { key }
    }

    pub fn issue_anon(&self) -> (String, String) {
        let mut token = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut token);
        let token = hex::encode(token);
        let mac = hex::encode(self.mac(format!("csrf-anon|{token}").as_bytes()));
        (format!("v1.{token}.{mac}"), token)
    }

    pub fn verify_anon(&self, cookie_value: &str, form_token: &str) -> bool {
        let parts = cookie_value.split('.').collect::<Vec<_>>();
        if parts.len() != 3 || parts[0] != "v1" {
            return false;
        }
        if !constant_time_eq(parts[1].as_bytes(), form_token.as_bytes()) {
            return false;
        }
        let expected = hex::encode(self.mac(format!("csrf-anon|{}", parts[1]).as_bytes()));
        constant_time_eq(parts[2].as_bytes(), expected.as_bytes())
    }

    pub fn for_session(&self, session_token: &str) -> String {
        hex::encode(self.mac(format!("csrf-session|{session_token}").as_bytes()))
    }

    pub fn verify_session(&self, session_token: &str, form_token: &str) -> bool {
        let expected = self.for_session(session_token);
        constant_time_eq(expected.as_bytes(), form_token.as_bytes())
    }

    fn mac(&self, data: &[u8]) -> [u8; 32] {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts 32-byte keys");
        mac.update(data);
        mac.finalize().into_bytes().into()
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in a.iter().zip(b) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csrf() -> Csrf {
        Csrf::from_hex(&"22".repeat(32)).unwrap()
    }

    #[test]
    fn anonymous_token_round_trip_and_tamper() {
        let csrf = csrf();
        let (cookie, token) = csrf.issue_anon();
        assert!(csrf.verify_anon(&cookie, &token));
        assert!(!csrf.verify_anon(&cookie, "other"));
        let mut tampered = cookie.clone();
        tampered.push('0');
        assert!(!csrf.verify_anon(&tampered, &token));
    }

    #[test]
    fn session_token_is_bound_to_session() {
        let csrf = csrf();
        let token = csrf.for_session("session-a");
        assert!(csrf.verify_session("session-a", &token));
        assert!(!csrf.verify_session("session-b", &token));
        assert!(!csrf.verify_session("session-a", "wrong"));
    }
}
