//! OpenPGP card applet (ISO/IEC 7816-4, OpenPGP Smart Card spec 3.x): uses the
//! decryption key (key reference B8) with ECDH on NIST P-256 or P-384.
use zeroize::Zeroizing;

use super::tlv::{self, find_deep};
use super::{command, status_error, Applet, CardKey, CardStatus, Curve, Transport};
use crate::error::{CoreError, CoreResult};

pub const AID: &[u8] = &[0xD2, 0x76, 0x00, 0x01, 0x24, 0x01];
/// Algorithm id for ECDH in the algorithm attributes (C2).
const ALG_ECDH: u8 = 0x12;

async fn select<T: Transport>(t: &mut T) -> CoreResult<bool> {
    let (_, sw) = command(t, 0x00, 0xA4, 0x04, 0x00, AID, true).await?;
    Ok(sw == 0x9000)
}

fn label(aid: Option<&[u8]>) -> String {
    let Some(aid) = aid.filter(|a| a.len() >= 14) else {
        return "OpenPGP card".into();
    };
    let maker = match ((aid[8] as u16) << 8) | aid[9] as u16 {
        0x0006 => "YubiKey",
        0x000F => "Nitrokey",
        0x0005 => "ZeitControl",
        _ => "OpenPGP card",
    };
    format!("{maker} {}", hex::encode_upper(&aid[10..14]))
}

/// The card's status, or None when it has no OpenPGP applet.
pub async fn status<T: Transport>(t: &mut T) -> CoreResult<Option<CardStatus>> {
    if !select(t).await? {
        return Ok(None);
    }
    let (ard, sw) = command(t, 0x00, 0xCA, 0x00, 0x6E, &[], true).await?;
    if sw != 0x9000 {
        return Err(status_error(sw, "OpenPGP data"));
    }
    let label = label(find_deep(&ard, 0x4F));
    let pin_tries = find_deep(&ard, 0xC4).and_then(|c| c.get(4)).copied();
    let curve = find_deep(&ard, 0xC2).and_then(|a| {
        if a.first() != Some(&ALG_ECDH) {
            return None;
        }
        // Some cards append an import-format byte after the OID.
        let oid = &a[1..];
        if oid.starts_with(Curve::OID_P384) {
            Some(Curve::P384)
        } else if oid.starts_with(Curve::OID_P256) {
            Some(Curve::P256)
        } else {
            None
        }
    });
    let mut key = None;
    if let Some(curve) = curve {
        let (pk, sw) = command(t, 0x00, 0x47, 0x81, 0x00, &[0xB8, 0x00], true).await?;
        if sw == 0x9000 {
            if let Some(point) = find_deep(&pk, 0x86).filter(|p| p.len() == curve.point_len()) {
                key = Some(CardKey {
                    applet: Applet::OpenPgp,
                    curve,
                    public: point.to_vec(),
                    label: label.clone(),
                });
            }
        }
    }
    Ok(Some(CardStatus {
        key,
        applet: Some(Applet::OpenPgp),
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
    let (_, sw) = command(t, 0x00, 0x20, 0x00, 0x82, pin, false).await?;
    if sw != 0x9000 {
        return Err(status_error(sw, "PIN"));
    }
    let data = tlv::encode(0xA6, &tlv::encode(0x7F49, &tlv::encode(0x86, peer)));
    let (shared, sw) = command(t, 0x00, 0x2A, 0x80, 0x86, &data, true).await?;
    if sw != 0x9000 {
        return Err(status_error(sw, "decryption key"));
    }
    let shared = Zeroizing::new(shared);
    if shared.len() != (key.curve.point_len() - 1) / 2 {
        return Err(CoreError::DecryptFailed);
    }
    Ok(shared)
}

/// Makes a new ECDH decryption key on the card (replacing any existing one)
/// with the admin PIN (factory default 12345678). Returns the new key.
pub async fn generate<T: Transport>(
    t: &mut T,
    admin_pin: &[u8],
    curve: Curve,
) -> CoreResult<CardKey> {
    let Some(st) = status(t).await? else {
        return Err(CoreError::InvalidInput {
            message: "this card has no OpenPGP applet".into(),
        });
    };
    let (_, sw) = command(t, 0x00, 0x20, 0x00, 0x83, admin_pin, false).await?;
    if sw != 0x9000 {
        return Err(status_error(sw, "admin PIN"));
    }
    let mut attrs = vec![ALG_ECDH];
    attrs.extend_from_slice(curve.oid());
    let (_, sw) = command(t, 0x00, 0xDA, 0x00, 0xC2, &attrs, false).await?;
    if sw != 0x9000 {
        return Err(status_error(sw, "support for this curve"));
    }
    let (pk, sw) = command(t, 0x00, 0x47, 0x80, 0x00, &[0xB8, 0x00], true).await?;
    if sw != 0x9000 {
        return Err(status_error(sw, "key generation"));
    }
    let point = find_deep(&pk, 0x86)
        .filter(|p| p.len() == curve.point_len())
        .ok_or(CoreError::InvalidInput {
            message: "the card made a key Atropos cannot read".into(),
        })?;
    Ok(CardKey {
        applet: Applet::OpenPgp,
        curve,
        public: point.to_vec(),
        label: st.label,
    })
}
