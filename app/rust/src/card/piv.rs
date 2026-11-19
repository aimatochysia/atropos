//! PIV applet (NIST SP 800-73-4): uses the key management key in slot 9D
//! with ECDH (algorithm 0x11 = P-256, 0x14 = P-384).
use zeroize::Zeroizing;

use super::tlv::{self, find, find_deep};
use super::{command, status_error, Applet, CardKey, CardStatus, Curve, Transport};
use crate::error::{CoreError, CoreResult};

pub const AID: &[u8] = &[0xA0, 0x00, 0x00, 0x03, 0x08];
const SLOT_KEY_MANAGEMENT: u8 = 0x9D;
const ID_EC_PUBLIC_KEY: &[u8] = &[0x06, 0x07, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];

fn alg(curve: Curve) -> u8 {
    match curve {
        Curve::P256 => 0x11,
        Curve::P384 => 0x14,
    }
}

async fn select<T: Transport>(t: &mut T) -> CoreResult<bool> {
    let (_, sw) = command(t, 0x00, 0xA4, 0x04, 0x00, AID, true).await?;
    Ok(sw == 0x9000)
}

/// The EC public point in an X.509 certificate's SubjectPublicKeyInfo.
fn cert_point(cert: &[u8]) -> Option<(Curve, Vec<u8>)> {
    let at = cert
        .windows(ID_EC_PUBLIC_KEY.len())
        .position(|w| w == ID_EC_PUBLIC_KEY)?;
    let rest = &cert[at + ID_EC_PUBLIC_KEY.len()..];
    let curve = if rest.get(2..2 + Curve::OID_P384.len()) == Some(Curve::OID_P384) {
        Curve::P384
    } else if rest.get(2..2 + Curve::OID_P256.len()) == Some(Curve::OID_P256) {
        Curve::P256
    } else {
        return None;
    };
    let n = curve.point_len();
    // BIT STRING: 03, length (n + 1), 00 unused bits, then the point.
    let pos = rest
        .windows(3)
        .position(|w| w[0] == 0x03 && w[1] as usize == n + 1 && w[2] == 0x00)?;
    let point = rest.get(pos + 3..pos + 3 + n)?;
    (point[0] == 0x04).then(|| (curve, point.to_vec()))
}

pub async fn status<T: Transport>(t: &mut T) -> CoreResult<Option<CardStatus>> {
    if !select(t).await? {
        return Ok(None);
    }
    // YubiKey serial (vendor command); other cards just say "PIV card".
    let (serial, sw) = command(t, 0x00, 0xF8, 0x00, 0x00, &[], true).await?;
    let label = if sw == 0x9000 && serial.len() == 4 {
        format!(
            "YubiKey {}",
            u32::from_be_bytes([serial[0], serial[1], serial[2], serial[3]])
        )
    } else {
        "PIV card".to_string()
    };
    let _ = select(t).await?;
    let mut found: Option<(Curve, Vec<u8>)> = None;
    // Key metadata (YubiKey 5.3+) gives the public key directly.
    let (meta, sw) = command(t, 0x00, 0xF7, 0x00, SLOT_KEY_MANAGEMENT, &[], true).await?;
    if sw == 0x9000 {
        let curve = match find(&meta, 0x01).and_then(|a| a.first()) {
            Some(0x11) => Some(Curve::P256),
            Some(0x14) => Some(Curve::P384),
            _ => None,
        };
        if let (Some(curve), Some(pk)) = (curve, find(&meta, 0x04)) {
            if let Some(point) = find(pk, 0x86).filter(|p| p.len() == curve.point_len()) {
                found = Some((curve, point.to_vec()));
            }
        }
    }
    if found.is_none() {
        // Otherwise the certificate stored for slot 9D.
        let (obj, sw) = command(
            t,
            0x00,
            0xCB,
            0x3F,
            0xFF,
            &[0x5C, 0x03, 0x5F, 0xC1, 0x0B],
            true,
        )
        .await?;
        if sw == 0x9000 {
            if let Some(cert) = find_deep(&obj, 0x70) {
                found = cert_point(cert);
            }
        }
    }
    let (_, sw) = command(t, 0x00, 0x20, 0x00, 0x80, &[], false).await?;
    let pin_tries = (sw & 0xFFF0 == 0x63C0).then_some((sw & 0x0F) as u8);
    Ok(Some(CardStatus {
        key: found.map(|(curve, public)| CardKey {
            applet: Applet::Piv,
            curve,
            public,
            label: label.clone(),
        }),
        applet: Some(Applet::Piv),
        pin_tries,
        label,
    }))
}

pub async fn agree<T: Transport>(
    t: &mut T,
    key: &CardKey,
    pin: &[u8],
    peer: &[u8],
) -> CoreResult<Zeroizing<Vec<u8>>> {
    if !select(t).await? {
        return Err(CoreError::InvalidInput {
            message: "this is not the card that was added".into(),
        });
    }
    if pin.len() < 6 || pin.len() > 8 {
        return Err(CoreError::InvalidInput {
            message: "a PIV PIN has 6 to 8 characters".into(),
        });
    }
    let mut padded = Zeroizing::new([0xFFu8; 8]);
    padded[..pin.len()].copy_from_slice(pin);
    let (_, sw) = command(t, 0x00, 0x20, 0x00, 0x80, padded.as_ref(), false).await?;
    if sw != 0x9000 {
        return Err(status_error(sw, "PIN"));
    }
    let mut inner = vec![0x82, 0x00];
    inner.extend_from_slice(&tlv::encode(0x85, peer));
    let data = tlv::encode(0x7C, &inner);
    let (resp, sw) = command(
        t,
        0x00,
        0x87,
        alg(key.curve),
        SLOT_KEY_MANAGEMENT,
        &data,
        true,
    )
    .await?;
    if sw != 0x9000 {
        return Err(status_error(sw, "key management key in slot 9D"));
    }
    let shared = find_deep(&resp, 0x82)
        .map(|s| Zeroizing::new(s.to_vec()))
        .ok_or(CoreError::DecryptFailed)?;
    if shared.len() != (key.curve.point_len() - 1) / 2 {
        return Err(CoreError::DecryptFailed);
    }
    Ok(shared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_point_in_a_certificate_like_blob() {
        let mut cert = vec![0x30, 0x82, 0x01, 0x00, 0x30, 0x10];
        cert.extend_from_slice(ID_EC_PUBLIC_KEY);
        cert.push(0x06);
        cert.push(Curve::OID_P384.len() as u8);
        cert.extend_from_slice(Curve::OID_P384);
        cert.extend_from_slice(&[0x03, 98, 0x00, 0x04]);
        cert.extend_from_slice(&[9u8; 96]);
        let (curve, point) = cert_point(&cert).unwrap();
        assert_eq!(curve, Curve::P384);
        assert_eq!(point.len(), 97);
        assert_eq!(point[0], 0x04);
    }
}
