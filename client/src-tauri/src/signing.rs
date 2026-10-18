//! Request signing, matching server/ems_solver/auth.py (see docs/api.md).

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

pub const VERSION: &str = "EMS1";

pub fn canonical(method: &str, path: &str, client_id: &str, timestamp: &str, nonce: &str, body: &[u8]) -> String {
    let body_hash = hex::encode(Sha256::digest(body));
    [VERSION, &method.to_uppercase(), path, client_id, timestamp, nonce, &body_hash].join("\n")
}

pub fn sign(secret: &str, method: &str, path: &str, client_id: &str, timestamp: &str, nonce: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(canonical(method, path, client_id, timestamp, nonce, body).as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_server_implementation() {
        // Computed with ems_solver.auth.sign on the server side.
        let body = br#"{"mode":"hint","parts":[{"type":"text","text":"2+2?"}]}"#;
        assert_eq!(
            sign("s3cret", "POST", "/v1/solve/stream", "tester", "1760000000", "abcdef0123456789abcdef", body),
            "621ae743ae3447dc4bdbe5f94557bc2559c0f84c9d60b591d3957c8d3dae335c"
        );
    }
}
