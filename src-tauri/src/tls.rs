//! Self-signed HTTPS (experimental `self_signed`): an account can pin its server's certificate.
//!
//! The user fetches the certificate's SHA-256 fingerprint (`cert_fingerprint`), compares it with the
//! one their NAS shows, and confirms; it's saved in the account (`Profile::cert_pin`). Requests to
//! that account then accept exactly that certificate (even self-signed, or for an IP address), and
//! anything else still goes through the normal system check. Never a blanket "skip verification".
//!
//! WebDAV/Nextcloud use reqwest with this verifier (`webdav::http_for`). S3 uses the AWS SDK, whose
//! TLS can't take a custom verifier, so pinned S3 accounts send their requests through reqwest too
//! (`PinnedHttpClient`).

use std::sync::{Arc, Mutex};

use aws_smithy_runtime_api::client::http::{
    HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
};
use aws_smithy_runtime_api::client::orchestrator::{HttpRequest, HttpResponse};
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
use aws_smithy_types::body::SdkBody;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// "AB:CD:…" — SHA-256 of the certificate, the way browsers and NAS admin pages show it.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Accepts the pinned certificate; everything else goes to the platform verifier. With `capture`,
/// accepts anything and remembers the server's certificate (only used to read the fingerprint).
#[derive(Debug)]
struct Verifier {
    pin: Option<String>,
    capture: Option<Arc<Mutex<Option<Vec<u8>>>>>,
    inner: Option<Arc<dyn ServerCertVerifier>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if let Some(c) = &self.capture {
            *c.lock().unwrap() = Some(end_entity.to_vec());
            return Ok(ServerCertVerified::assertion());
        }
        if self.pin.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(&fingerprint(end_entity))) {
            return Ok(ServerCertVerified::assertion());
        }
        match &self.inner {
            Some(v) => v.verify_server_cert(end_entity, intermediates, server_name, ocsp, now),
            None => Err(rustls::Error::General("certificate not trusted".into())),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn config(v: Verifier) -> Result<rustls::ClientConfig, String> {
    let p = v.provider.clone();
    Ok(rustls::ClientConfig::builder_with_provider(p)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(v))
        .with_no_client_auth())
}

/// TLS settings that trust `pin` (plus everything the system trusts).
pub fn pinned_config(pin: &str) -> Result<rustls::ClientConfig, String> {
    let provider = provider();
    let inner = rustls_platform_verifier::Verifier::new(provider.clone()).map_err(|e| e.to_string())?;
    config(Verifier { pin: Some(pin.to_string()), capture: None, inner: Some(Arc::new(inner)), provider })
}

/// A reqwest client for an account with a pinned certificate.
pub fn pinned_client(pin: &str) -> Result<reqwest::Client, String> {
    crate::webdav::http_builder().use_preconfigured_tls(pinned_config(pin)?).build().map_err(|e| e.to_string())
}

/// The fingerprint of the certificate `endpoint` presents, read without trusting it, so the user
/// can compare it before pinning. Fails when the server has no HTTPS.
#[tauri::command]
pub async fn cert_fingerprint(endpoint: String) -> Result<String, String> {
    let url = crate::config::normalize_endpoint(&endpoint);
    if !url.starts_with("https://") {
        return Err("Only https:// addresses have a certificate".into());
    }
    let seen = Arc::new(Mutex::new(None));
    let v = Verifier { pin: None, capture: Some(seen.clone()), inner: None, provider: provider() };
    let client = crate::webdav::http_builder().use_preconfigured_tls(config(v)?).build().map_err(|e| e.to_string())?;
    // Any answer will do (even 403): the handshake is what we're after.
    let _ = client.head(&url).send().await;
    let der = seen.lock().unwrap().take().ok_or("Couldn't reach the server over HTTPS")?;
    Ok(fingerprint(&der))
}

// ---------- S3 through reqwest (pinned accounts only) ----------

/// The AWS SDK's HTTP client, swapped for reqwest with the pinned certificate.
#[derive(Debug, Clone)]
pub struct PinnedHttpClient {
    client: reqwest::Client,
}

impl PinnedHttpClient {
    pub fn new(pin: &str) -> Result<Self, String> {
        Ok(Self { client: pinned_client(pin)? })
    }
}

impl HttpClient for PinnedHttpClient {
    fn http_connector(&self, _: &HttpConnectorSettings, _: &RuntimeComponents) -> SharedHttpConnector {
        SharedHttpConnector::new(self.clone())
    }
}

impl HttpConnector for PinnedHttpClient {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let client = self.client.clone();
        HttpConnectorFuture::new(async move {
            let req = request.try_into_http1x().map_err(|e| ConnectorError::other(e.into(), None))?;
            let (parts, body) = req.into_parts();
            let resp = client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(reqwest::Body::wrap(body))
                .send()
                .await
                .map_err(|e| {
                    if e.is_timeout() { ConnectorError::timeout(e.into()) } else { ConnectorError::io(e.into()) }
                })?;
            let (parts, body) = http::Response::<reqwest::Body>::from(resp).into_parts();
            let resp = http::Response::from_parts(parts, SdkBody::from_body_1_x(body));
            HttpResponse::try_from(resp).map_err(|e| ConnectorError::other(e.into(), None))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_format() {
        let f = fingerprint(b"hello");
        assert_eq!(f.len(), 32 * 3 - 1);
        assert!(f.starts_with("2C:F2:4D:BA"));
    }

    /// Against an HTTPS proxy with an untrusted certificate in front of dev S3 and dev Nextcloud:
    /// `NOEXPLORER_TEST_TLS_S3=https://localhost:8443 NOEXPLORER_TEST_TLS_DAV=https://localhost:8444`
    /// (plus the usual S3 key/secret and DAV user/pass variables). See docs/DEVELOPMENT.md.
    #[tokio::test]
    async fn pinning_against_untrusted_proxy() {
        use crate::config::{Profile, ProviderKind};
        use crate::provider::Remote;
        let (Ok(s3_url), Ok(dav_url)) =
            (std::env::var("NOEXPLORER_TEST_TLS_S3"), std::env::var("NOEXPLORER_TEST_TLS_DAV"))
        else {
            eprintln!("skipping pinning test: NOEXPLORER_TEST_TLS_S3 / _DAV not set");
            return;
        };
        let s3 = |pin: &str| Profile {
            endpoint: s3_url.clone(),
            access_key: std::env::var("NOEXPLORER_TEST_KEY").unwrap(),
            secret_key: std::env::var("NOEXPLORER_TEST_SECRET").unwrap(),
            cert_pin: pin.into(),
            ..Default::default()
        };
        let dav = |pin: &str| Profile {
            kind: ProviderKind::Nextcloud,
            endpoint: dav_url.clone(),
            access_key: std::env::var("NOEXPLORER_TEST_DAV_USER").unwrap(),
            secret_key: std::env::var("NOEXPLORER_TEST_DAV_PASS").unwrap(),
            cert_pin: pin.into(),
            ..Default::default()
        };
        // Not trusted by default.
        assert!(Remote::new(&s3("")).containers().await.is_err(), "S3 must refuse the certificate");
        assert!(Remote::new(&dav("")).containers().await.is_err(), "WebDAV must refuse the certificate");
        // Pinned: works.
        let pin = cert_fingerprint(s3_url.clone()).await.unwrap();
        assert!(!Remote::new(&s3(&pin)).containers().await.unwrap().is_empty());
        let dpin = cert_fingerprint(dav_url.clone()).await.unwrap();
        assert_eq!(Remote::new(&dav(&dpin)).containers().await.unwrap()[0].name, "Files");
        // Real traffic through the swapped HTTP client: a small and a multipart upload, a download.
        let r = Remote::new(&s3(&pin));
        let dir = std::env::temp_dir().join(format!("nx-tls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let data: Vec<u8> = (0..(17 * 1024 * 1024 + 3)).map(|i| (i * 31 % 251) as u8).collect();
        std::fs::write(dir.join("big.bin"), &data).unwrap();
        std::fs::write(dir.join("small.txt"), "pinned").unwrap();
        let job = crate::jobs::Job::untracked("up", "t".into(), "", "noexplorer-test", "");
        let bucket = "noexplorer-test";
        r.upload_file(bucket, "tls/small.txt", &dir.join("small.txt"), 6, &job).await.unwrap();
        r.upload_file(bucket, "tls/big.bin", &dir.join("big.bin"), data.len() as u64, &job).await.unwrap();
        r.download_file(bucket, "tls/big.bin", &dir.join("out.bin"), &job).await.unwrap();
        assert!(std::fs::read(dir.join("out.bin")).unwrap() == data, "round trip through pinned TLS differs");
        r.delete(bucket, vec!["tls/".into()], None).await.unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        // A different certificate is still refused.
        let wrong = fingerprint(b"another certificate");
        assert!(Remote::new(&s3(&wrong)).containers().await.is_err());
        assert!(Remote::new(&dav(&wrong)).containers().await.is_err());
    }
}
