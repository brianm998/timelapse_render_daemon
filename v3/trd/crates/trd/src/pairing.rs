// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Pairing codes and device registration (§8.3).
//!
//! `StartPairing` (loopback only) issues one six-digit code at a time. `Pair` on either listener
//! redeems it: a correct, unexpired code creates a device and is consumed. Five wrong codes for the
//! current code invalidate it. Time is passed in, so the tests use a fake clock.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::rngs::OsRng;
use rand::{Rng, RngCore};
use trd_core::store::Device;

use crate::auth::token_hash;

/// How long a code stays valid: five minutes (§8.3).
pub const CODE_LIFETIME_MS: i64 = 5 * 60 * 1000;
/// The wrong codes that invalidate the current code (§8.3).
pub const MAX_WRONG_CODES: u32 = 5;

/// A code as `StartPairing` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedCode {
    pub code: String,
    pub expires_at_ms: i64,
}

/// Why a `Pair` was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No code is held, or the held one has expired.
    NoCode,
    /// The code is wrong, and more wrong codes are still allowed.
    Wrong,
    /// The code was wrong for the last time: it is invalid now.
    TooManyWrong,
}

/// The one pairing code that can be valid at a time.
#[derive(Debug, Default)]
pub struct Pairing {
    pending: Option<Pending>,
}

#[derive(Debug)]
struct Pending {
    code: String,
    expires_at_ms: i64,
    wrong: u32,
}

impl Pairing {
    /// Replaces any held code with `code`, which lasts [`CODE_LIFETIME_MS`] from `now_ms`.
    pub fn issue(&mut self, now_ms: i64, code: String) -> IssuedCode {
        let expires_at_ms = now_ms.saturating_add(CODE_LIFETIME_MS);
        self.pending = Some(Pending {
            code: code.clone(),
            expires_at_ms,
            wrong: 0,
        });
        IssuedCode {
            code,
            expires_at_ms,
        }
    }

    /// Redeems `presented` at `now_ms`. The correct code is consumed. A wrong one counts toward
    /// [`MAX_WRONG_CODES`], and the last of those clears the code.
    pub fn redeem(&mut self, now_ms: i64, presented: &str) -> Result<(), Refusal> {
        let Some(pending) = self.pending.as_mut() else {
            return Err(Refusal::NoCode);
        };
        if now_ms >= pending.expires_at_ms {
            self.pending = None;
            return Err(Refusal::NoCode);
        }
        if same_code(&pending.code, presented) {
            self.pending = None;
            return Ok(());
        }
        pending.wrong += 1;
        if pending.wrong >= MAX_WRONG_CODES {
            self.pending = None;
            return Err(Refusal::TooManyWrong);
        }
        Err(Refusal::Wrong)
    }
}

/// Compares two codes in time that does not depend on where they differ.
fn same_code(held: &str, presented: &str) -> bool {
    held.len() == presented.len()
        && held
            .bytes()
            .zip(presented.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// A fresh six-digit code from the operating system's random source.
pub fn new_code() -> String {
    let number = OsRng.gen_range(0..1_000_000u32);
    format!("{number:06}")
}

/// A fresh device token: 32 random bytes, base64url without padding.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The device a successful `Pair` creates, with the token it returns. Only the token's hash is in
/// the device row (§8.3).
pub fn new_device(name: &str, platform: &str, now_ms: i64) -> (Device, String) {
    let token = new_token();
    let device = Device {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_owned(),
        platform: platform.to_owned(),
        token_sha256: token_hash(&token),
        paired_at: now_ms,
        last_seen: None,
        revoked: false,
    };
    (device, token)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_000_000;

    fn issued(pairing: &mut Pairing, code: &str) {
        pairing.issue(T0, code.to_owned());
    }

    #[test]
    fn a_new_code_is_six_digits() {
        for _ in 0..50 {
            let code = new_code();
            assert_eq!(code.len(), 6);
            assert!(code.bytes().all(|b| b.is_ascii_digit()), "{code}");
        }
    }

    #[test]
    fn the_correct_code_is_consumed() {
        let mut pairing = Pairing::default();
        issued(&mut pairing, "123456");
        assert_eq!(pairing.redeem(T0, "123456"), Ok(()));
        assert_eq!(pairing.redeem(T0, "123456"), Err(Refusal::NoCode));
    }

    /// Code expiry on a fake clock: valid just before the lifetime, refused at it.
    #[test]
    fn a_code_expires_after_its_lifetime() {
        let mut early = Pairing::default();
        let issued_code = early.issue(T0, "654321".to_owned());
        assert_eq!(issued_code.expires_at_ms, T0 + CODE_LIFETIME_MS);
        assert_eq!(early.redeem(T0 + CODE_LIFETIME_MS - 1, "654321"), Ok(()));

        let mut late = Pairing::default();
        late.issue(T0, "654321".to_owned());
        assert_eq!(
            late.redeem(T0 + CODE_LIFETIME_MS, "654321"),
            Err(Refusal::NoCode)
        );
    }

    /// A re-issued code replaces the old one, and the attempt count starts again.
    #[test]
    fn a_reissued_code_replaces_the_old_one() {
        let mut pairing = Pairing::default();
        issued(&mut pairing, "111111");
        assert_eq!(pairing.redeem(T0, "000000"), Err(Refusal::Wrong));
        issued(&mut pairing, "222222");
        assert_eq!(pairing.redeem(T0, "111111"), Err(Refusal::Wrong));
        assert_eq!(pairing.redeem(T0, "222222"), Ok(()));
    }

    /// A3: the fifth wrong code invalidates the code, and the correct one is then refused.
    #[test]
    fn five_wrong_codes_invalidate_the_code() {
        let mut pairing = Pairing::default();
        issued(&mut pairing, "424242");
        for _ in 0..MAX_WRONG_CODES - 1 {
            assert_eq!(pairing.redeem(T0, "000000"), Err(Refusal::Wrong));
        }
        assert_eq!(pairing.redeem(T0, "000000"), Err(Refusal::TooManyWrong));
        assert_eq!(pairing.redeem(T0, "424242"), Err(Refusal::NoCode));
    }

    #[test]
    fn a_code_of_another_length_is_wrong() {
        let mut pairing = Pairing::default();
        issued(&mut pairing, "123456");
        assert_eq!(pairing.redeem(T0, "12345"), Err(Refusal::Wrong));
        assert_eq!(pairing.redeem(T0, "1234567"), Err(Refusal::Wrong));
    }

    #[test]
    fn no_code_is_refused() {
        assert_eq!(
            Pairing::default().redeem(T0, "123456"),
            Err(Refusal::NoCode)
        );
    }

    /// Hash-only token storage: the device row holds the token's SHA-256, never the token.
    #[test]
    fn a_device_stores_only_the_token_hash() {
        let (device, token) = new_device("Phone", "ios", T0);
        assert_eq!(device.token_sha256, token_hash(&token));
        assert_ne!(device.token_sha256, token);
        assert!(!device.token_sha256.contains(&token));
        assert_eq!(device.paired_at, T0);
        assert_eq!(device.last_seen, None);
        assert!(!device.revoked);
        assert!(uuid::Uuid::parse_str(&device.id).is_ok());
        assert_eq!(token.len(), 43, "32 bytes in base64url without padding");
    }

    #[test]
    fn tokens_differ() {
        assert_ne!(new_token(), new_token());
    }

    #[test]
    fn same_code_compares_every_byte() {
        assert!(same_code("123456", "123456"));
        assert!(!same_code("123456", "123457"));
        assert!(!same_code("123456", "023456"));
    }
}
