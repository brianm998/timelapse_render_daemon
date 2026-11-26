// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Pinned TLS for the LAN listener (§8.1).
//!
//! [`load_or_create`] returns the identity the LAN listener serves: a self-signed ECDSA P-256
//! certificate and its PKCS#8 key, kept in `identity/` under the data directory. The fingerprint is
//! the lowercase hex SHA-256 of the certificate's DER, so it is the same after every restart: the
//! files are reused, never regenerated. The DER is serialized once; the PEM text and the fingerprint
//! both come from those bytes.

use std::fs;
use std::io::Write;
use std::net::IpAddr;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rcgen::{CertificateParams, DnType, KeyPair, PKCS_ECDSA_P256_SHA256};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

/// How long the certificate is valid: 20 years (§8.1).
pub const VALIDITY_DAYS: i64 = 365 * 20;

/// The LAN listener's certificate and key.
#[derive(Debug, Clone)]
pub struct Identity {
    pub cert_pem: String,
    pub key_pem: String,
    /// Lowercase hex SHA-256 of the certificate's DER.
    pub fingerprint: String,
}

/// Lowercase hex SHA-256 of a certificate's DER (§8.1).
pub fn fingerprint_of(der: &[u8]) -> String {
    hex::encode(Sha256::digest(der))
}

/// The names the certificate covers: the advertised name as a DNS label, `<label>.local`,
/// `localhost` and every non-loopback IPv4 address of this machine.
pub fn subject_names(advertised: &str) -> Vec<String> {
    let label: String = advertised
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let label = label.trim_matches('-');
    let label = if label.is_empty() { "trd" } else { label };
    let mut names = vec![
        label.to_owned(),
        format!("{label}.local"),
        "localhost".to_owned(),
    ];
    names.extend(lan_ipv4_addresses());
    names
}

/// The non-loopback IPv4 addresses of this machine, sorted, without duplicates.
pub fn lan_ipv4_addresses() -> Vec<String> {
    let networks = sysinfo::Networks::new_with_refreshed_list();
    let mut out: Vec<String> = networks
        .values()
        .flat_map(|data| data.ip_networks().iter())
        .filter_map(|network| match network.addr {
            IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_unspecified() => Some(v4.to_string()),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Loads the identity from `data/identity/`, or creates it when the certificate or key is missing.
/// `names` are the subject alternative names of a new certificate.
pub fn load_or_create(data: &Path, names: &[String]) -> Result<Identity, String> {
    let dir = data.join("identity");
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    let fingerprint_path = dir.join("fingerprint.txt");
    if cert_path.exists() && key_path.exists() {
        let cert_pem = read_text(&cert_path)?;
        let key_pem = read_text(&key_path)?;
        let cert_der = der_of(&cert_pem)
            .ok_or_else(|| format!("{} holds no certificate", cert_path.display()))?;
        let fingerprint = fingerprint_of(&cert_der);
        let recorded = fs::read_to_string(&fingerprint_path).unwrap_or_default();
        if recorded.trim() != fingerprint {
            write_file(&fingerprint_path, &format!("{fingerprint}\n"), false)?;
        }
        return Ok(Identity {
            cert_pem,
            key_pem,
            fingerprint,
        });
    }

    fs::create_dir_all(&dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .map_err(|err| format!("cannot generate the TLS key: {err}"))?;
    let mut params = CertificateParams::new(names.to_vec())
        .map_err(|err| format!("cannot name the TLS certificate: {err}"))?;
    params.distinguished_name.push(DnType::CommonName, "trd");
    let now = OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + Duration::days(VALIDITY_DAYS);
    let certificate = params
        .self_signed(&key)
        .map_err(|err| format!("cannot create the TLS certificate: {err}"))?;

    let cert_der = certificate.der().to_vec();
    let key_der = key.serialize_der();
    let cert_pem = pem("CERTIFICATE", &cert_der);
    let key_pem = pem("PRIVATE KEY", &key_der);
    let fingerprint = fingerprint_of(&cert_der);
    write_file(&cert_path, &cert_pem, false)?;
    write_file(&key_path, &key_pem, true)?;
    write_file(&fingerprint_path, &format!("{fingerprint}\n"), false)?;
    Ok(Identity {
        cert_pem,
        key_pem,
        fingerprint,
    })
}

/// A PEM block of `der` under `label`, with lines of 64 characters.
fn pem(label: &str, der: &[u8]) -> String {
    let body = STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in body.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

/// The DER inside the first PEM block of `text`.
fn der_of(text: &str) -> Option<Vec<u8>> {
    let body: String = text
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    STANDARD.decode(body).ok()
}

fn read_text(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|err| format!("cannot read {}: {err}", path.display()))
}

/// Writes `text` to `path`. A private file (the key) is created with mode 0600 on unix.
fn write_file(path: &Path, text: &str, private: bool) -> Result<(), String> {
    let fail = |err: std::io::Error| format!("cannot write {}: {err}", path.display());
    if private {
        write_private(path, text).map_err(fail)
    } else {
        fs::write(path, text).map_err(fail)
    }
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("trd-tls-unit-")
            .tempdir()
            .expect("a temporary directory")
    }

    fn names() -> Vec<String> {
        subject_names("Studio Mac")
    }

    #[test]
    fn a_new_identity_is_written_and_its_fingerprint_is_the_der_hash() {
        let dir = temp();
        let identity = load_or_create(dir.path(), &names()).expect("an identity");
        assert_eq!(identity.fingerprint.len(), 64);
        let der = der_of(&identity.cert_pem).expect("the certificate's DER");
        assert_eq!(identity.fingerprint, fingerprint_of(&der));
        let recorded = fs::read_to_string(dir.path().join("identity/fingerprint.txt"))
            .expect("fingerprint.txt");
        assert_eq!(recorded.trim(), identity.fingerprint);
    }

    /// A.5: the files are reused, so the fingerprint and the PEM text do not change.
    #[test]
    fn an_existing_identity_is_reused() {
        let dir = temp();
        let first = load_or_create(dir.path(), &names()).expect("an identity");
        let second = load_or_create(dir.path(), &names()).expect("the same identity");
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(first.cert_pem, second.cert_pem);
        assert_eq!(first.key_pem, second.key_pem);
    }

    /// A fingerprint file that disagrees with the certificate is corrected, not trusted.
    #[test]
    fn a_wrong_fingerprint_file_is_rewritten() {
        let dir = temp();
        let first = load_or_create(dir.path(), &names()).expect("an identity");
        let path = dir.path().join("identity/fingerprint.txt");
        fs::write(&path, "stale\n").expect("the stale file");
        let again = load_or_create(dir.path(), &names()).expect("the identity");
        assert_eq!(again.fingerprint, first.fingerprint);
        assert_eq!(
            fs::read_to_string(&path).expect("the file").trim(),
            first.fingerprint
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp();
        load_or_create(dir.path(), &names()).expect("an identity");
        let mode = fs::metadata(dir.path().join("identity/key.pem"))
            .expect("key.pem")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn subject_names_are_dns_labels_and_localhost() {
        let names = subject_names("Studio Mac.local");
        assert_eq!(names[0], "studio-mac-local");
        assert_eq!(names[1], "studio-mac-local.local");
        assert_eq!(names[2], "localhost");
        assert_eq!(subject_names("  ")[0], "trd");
        assert!(lan_ipv4_addresses().iter().all(|ip| ip != "127.0.0.1"));
    }

    #[test]
    fn pem_round_trips_through_der_of() {
        let der: Vec<u8> = (0..200u8).collect();
        let text = pem("TEST", &der);
        assert!(text.starts_with("-----BEGIN TEST-----\n"));
        assert_eq!(der_of(&text), Some(der));
    }
}
