//! SCRAM (RFC 5802, RFC 7677) for MQTT v5 enhanced authentication: the
//! client side of SCRAM-SHA-1, SCRAM-SHA-256 and SCRAM-SHA-512, without
//! channel binding.
//!
//! The exchange (MQTT v5 §4.12):
//! 1. CONNECT carries the method and the client-first message
//!    `n,,n=<user>,r=<nonce>` as authentication data;
//! 2. the broker answers with AUTH (continue) carrying the server-first
//!    message `r=<nonce>,s=<salt>,i=<iterations>`; [`Scram::respond`] gives
//!    the client-final message `c=biws,r=<nonce>,p=<proof>`;
//! 3. the broker accepts with CONNACK carrying the server-final message
//!    `v=<signature>`, which [`Scram::verify_connack`] checks: a broker that
//!    does not know the password cannot produce it. (A broker that sends it
//!    in another AUTH instead is answered with empty data.)
//!
//! The password is used as UTF-8 bytes without SASLprep, which is the same
//! for ASCII passwords. The client-first message, and with it the nonce, is
//! fixed for the life of the connect options, so automatic reconnects
//! repeat it; the broker's nonce still makes every exchange unique.

use std::sync::Mutex;

use ring::{digest, hmac, pbkdf2, rand::SecureRandom};

use crate::base64;

/// Server-first messages asking for more iterations are refused: PBKDF2 runs
/// on the client's event loop.
pub const MAX_ITERATIONS: u32 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    Sha1,
    Sha256,
    Sha512,
}

impl Mechanism {
    /// The mechanism of an authentication method name (any case).
    pub fn parse(method: &str) -> Option<Self> {
        match method.to_ascii_uppercase().as_str() {
            "SCRAM-SHA-1" => Some(Self::Sha1),
            "SCRAM-SHA-256" => Some(Self::Sha256),
            "SCRAM-SHA-512" => Some(Self::Sha512),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Sha1 => "SCRAM-SHA-1",
            Self::Sha256 => "SCRAM-SHA-256",
            Self::Sha512 => "SCRAM-SHA-512",
        }
    }

    fn hmac(self) -> hmac::Algorithm {
        match self {
            Self::Sha1 => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            Self::Sha256 => hmac::HMAC_SHA256,
            Self::Sha512 => hmac::HMAC_SHA512,
        }
    }

    fn digest(self) -> &'static digest::Algorithm {
        match self {
            Self::Sha1 => &digest::SHA1_FOR_LEGACY_USE_ONLY,
            Self::Sha256 => &digest::SHA256,
            Self::Sha512 => &digest::SHA512,
        }
    }

    fn pbkdf2(self) -> pbkdf2::Algorithm {
        match self {
            Self::Sha1 => pbkdf2::PBKDF2_HMAC_SHA1,
            Self::Sha256 => pbkdf2::PBKDF2_HMAC_SHA256,
            Self::Sha512 => pbkdf2::PBKDF2_HMAC_SHA512,
        }
    }

    fn len(self) -> usize {
        self.digest().output_len()
    }
}

/// Where the exchange stands.
#[derive(Default)]
struct Progress {
    /// The server signature the server-final message must carry.
    expected: Option<Vec<u8>>,
    /// The server-final message came in an AUTH packet and was right.
    verified: bool,
}

/// One client's SCRAM credentials and exchange.
pub struct Scram {
    mech: Mechanism,
    password: Vec<u8>,
    client_first_bare: String,
    nonce: String,
    progress: Mutex<Progress>,
}

impl std::fmt::Debug for Scram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scram").field("mech", &self.mech).finish_non_exhaustive()
    }
}

/// `=` and `,` in a user name (RFC 5802 saslname).
fn saslname(user: &str) -> String {
    user.replace('=', "=3D").replace(',', "=2C")
}

fn hmac(mech: Mechanism, key: &[u8], data: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(mech.hmac(), key), data).as_ref().to_vec()
}

impl Scram {
    /// A new exchange with a random nonce.
    pub fn new(mech: Mechanism, user: &str, password: &[u8]) -> Result<Self, String> {
        let mut raw = [0u8; 24];
        ring::rand::SystemRandom::new()
            .fill(&mut raw)
            .map_err(|_| "no random numbers for the SCRAM nonce".to_string())?;
        Ok(Self::with_nonce(mech, user, password, &base64::encode(&raw)))
    }

    /// An exchange with a given nonce (printable, without `,`; tests).
    pub fn with_nonce(mech: Mechanism, user: &str, password: &[u8], nonce: &str) -> Self {
        Self {
            mech,
            password: password.to_vec(),
            client_first_bare: format!("n={},r={nonce}", saslname(user)),
            nonce: nonce.to_string(),
            progress: Mutex::new(Progress::default()),
        }
    }

    pub fn mechanism(&self) -> Mechanism {
        self.mech
    }

    /// The client-first message, the authentication data of CONNECT.
    pub fn client_first(&self) -> Vec<u8> {
        format!("n,,{}", self.client_first_bare).into_bytes()
    }

    fn progress(&self) -> std::sync::MutexGuard<'_, Progress> {
        self.progress.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Answers the broker's authentication data: the server-first message
    /// gives the client-final message; a server-final message is checked and
    /// answered with nothing.
    pub fn respond(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        if data.starts_with(b"v=") || data.starts_with(b"e=") {
            self.verify_final(data)?;
            self.progress().verified = true;
            return Ok(Vec::new());
        }
        let server_first = std::str::from_utf8(data).map_err(|_| "server-first message is not UTF-8".to_string())?;
        let (mut nonce, mut salt, mut iterations) = (None, None, None);
        for (i, attr) in server_first.split(',').enumerate() {
            match attr.split_at_checked(2) {
                Some(("m=", _)) if i == 0 => return Err("the server requires an unsupported SCRAM extension".into()),
                Some(("r=", v)) => nonce = Some(v),
                Some(("s=", v)) => salt = Some(base64::decode(v.as_bytes()).map_err(|e| format!("salt: {e}"))?),
                Some(("i=", v)) => iterations = v.parse::<u32>().ok(),
                _ => {}
            }
        }
        let (Some(nonce), Some(salt), Some(iterations)) = (nonce, salt, iterations) else {
            return Err(format!("malformed server-first message '{server_first}'"));
        };
        if !nonce.starts_with(&self.nonce) || nonce.len() == self.nonce.len() {
            return Err("the server's nonce does not continue the client's".into());
        }
        if iterations == 0 || iterations > MAX_ITERATIONS {
            return Err(format!("iteration count {iterations} out of range (1..{MAX_ITERATIONS})"));
        }

        let m = self.mech;
        let mut salted = vec![0u8; m.len()];
        pbkdf2::derive(
            m.pbkdf2(),
            std::num::NonZeroU32::new(iterations).expect("checked above"),
            &salt,
            &self.password,
            &mut salted,
        );
        let client_key = hmac(m, &salted, b"Client Key");
        let stored_key = digest::digest(m.digest(), &client_key);
        let without_proof = format!("c=biws,r={nonce}");
        let auth_message = format!("{},{server_first},{without_proof}", self.client_first_bare);
        let signature = hmac(m, stored_key.as_ref(), auth_message.as_bytes());
        let proof: Vec<u8> = client_key.iter().zip(&signature).map(|(a, b)| a ^ b).collect();
        let server_key = hmac(m, &salted, b"Server Key");

        let mut p = self.progress();
        p.expected = Some(hmac(m, &server_key, auth_message.as_bytes()));
        p.verified = false;
        Ok(format!("{without_proof},p={}", base64::encode(&proof)).into_bytes())
    }

    fn verify_final(&self, data: &[u8]) -> Result<(), String> {
        let text = String::from_utf8_lossy(data);
        if let Some(e) = text.strip_prefix("e=") {
            return Err(format!("the server refused the authentication: {e}"));
        }
        let v = text
            .split(',')
            .find_map(|a| a.strip_prefix("v="))
            .ok_or_else(|| format!("malformed server-final message '{text}'"))?;
        let got = base64::decode(v.as_bytes()).map_err(|e| format!("server signature: {e}"))?;
        let expected = self.progress().expected.clone().ok_or("server-final message before server-first")?;
        // not secret: comparing in constant time is not needed here
        if got != expected {
            return Err("wrong server signature: the broker does not know the password".into());
        }
        Ok(())
    }

    /// Checks the server-final message of a CONNACK's authentication data.
    pub fn verify_connack(&self, data: Option<&[u8]>) -> Result<(), String> {
        match data {
            Some(d) if !d.is_empty() => self.verify_final(d),
            _ if self.progress().verified => Ok(()),
            _ if self.progress().expected.is_none() => {
                Err("the broker accepted the connection without the SCRAM exchange".into())
            }
            _ => Err("the broker sent no server signature (server-final message)".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 5802, section 5
    #[test]
    fn sha1_rfc5802() {
        let s = Scram::with_nonce(Mechanism::Sha1, "user", b"pencil", "fyko+d2lbbFgONRv9qkxdawL");
        assert_eq!(s.client_first(), b"n,,n=user,r=fyko+d2lbbFgONRv9qkxdawL");
        let fin = s.respond(b"r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,s=QSXCR+Q6sek8bf92,i=4096").unwrap();
        assert_eq!(
            String::from_utf8(fin).unwrap(),
            "c=biws,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,p=v0X8v3Bz2T0CJGbJQyF0X+HI4Ts="
        );
        s.verify_connack(Some(b"v=rmF9pqV8S7suAoZWja4dJRkFsKQ=")).unwrap();
        assert!(s.verify_connack(Some(b"v=AAAAAAAAAAAAAAAAAAAAAAAAAAA=")).is_err());
        assert!(s.verify_connack(None).is_err());
    }

    /// RFC 7677, section 3
    #[test]
    fn sha256_rfc7677() {
        let s = Scram::with_nonce(Mechanism::Sha256, "user", b"pencil", "rOprNGfwEbeRWgbNEkqO");
        let fin = s
            .respond(b"r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096")
            .unwrap();
        assert_eq!(
            String::from_utf8(fin).unwrap(),
            "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
        );
        // the server-final message in an AUTH packet, then a CONNACK without data
        assert_eq!(s.respond(b"v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=").unwrap(), b"");
        s.verify_connack(None).unwrap();
    }

    #[test]
    fn refusals() {
        let s = Scram::with_nonce(Mechanism::Sha512, "a=b,c", b"pw", "abc");
        assert_eq!(s.client_first(), b"n,,n=a=3Db=2Cc,r=abc");
        assert!(s.verify_connack(None).unwrap_err().contains("without the SCRAM exchange"));
        for bad in [
            &b"r=xyz123,s=QSXCR+Q6sek8bf92,i=4096"[..], // nonce of someone else
            b"r=abc,s=QSXCR+Q6sek8bf92,i=4096",         // nothing added by the server
            b"r=abc1,s=QSXCR+Q6sek8bf92,i=0",
            b"r=abc1,s=QSXCR+Q6sek8bf92,i=2000000",
            b"r=abc1,s=!!,i=4096",
            b"r=abc1,i=4096",
            b"m=ext,r=abc1,s=QSXCR+Q6sek8bf92,i=4096",
        ] {
            assert!(s.respond(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        assert!(s.respond(b"e=invalid-proof").unwrap_err().contains("invalid-proof"));
        assert_eq!(Mechanism::parse("scram-sha-256"), Some(Mechanism::Sha256));
        assert_eq!(Mechanism::parse("SCRAM-SHA-384"), None);
    }
}
