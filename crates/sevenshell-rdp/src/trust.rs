//! Reading the certificate an RDP server presents, and putting it into the shape the vault keeps.
//!
//! RDP has no SSH host key; what stands in for it is the TLS certificate the server shows during
//! the security upgrade. Verification is by fingerprint against a value a person accepted, which
//! is what [`crate::probe`] exists to put in front of them.

use base64::Engine as _;
use sha2::{Digest, Sha256};
use x509_cert::der::Encode as _;

use sevenshell_error::ShellError;
use sevenshell_types::PresentedCertificate;

use crate::Target;
use crate::engine;

/// The digest a person compares: `SHA256:` and the base64 of the DER certificate's SHA-256.
///
/// The spelling matches the SSH fingerprints this application already shows, so the two kinds of
/// trust prompt read the same way.
pub(crate) fn fingerprint_parts(cert: &x509_cert::Certificate) -> Option<(String, String)> {
    let der = cert.to_der().ok()?;
    let digest = Sha256::digest(&der);
    let fingerprint = format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    );
    let certificate = base64::engine::general_purpose::STANDARD.encode(der);
    Some((fingerprint, certificate))
}

pub(crate) async fn probe(target: &Target) -> Result<PresentedCertificate, ShellError> {
    let cert = tokio::time::timeout(std::time::Duration::from_secs(20), engine::probe_certificate(target))
        .await
        .map_err(|_| {
            ShellError::rdp_unreachable(
                &target.host,
                target.port,
                "the server did not present a certificate in time",
            )
        })??;

    let (fingerprint, certificate) = fingerprint_parts(&cert)
        .ok_or_else(|| ShellError::rdp_tls_failed(&target.host, target.port, "the certificate could not be read"))?;

    Ok(PresentedCertificate {
        host: target.host.clone(),
        port: target.port,
        fingerprint,
        certificate,
    })
}
