//! Smartcards as vault keys: OpenPGP cards and PIV cards (YubiKey, Nitrokey,
//! many government and corporate cards), over contact readers (PC/SC,
//! ISO 7816) or NFC (ISO/IEC 14443, through the platform).
//!
//! The card holds an elliptic-curve private key (NIST P-384 or P-256) that
//! never leaves it. When a card is added to a vault, Atropos makes a one-off
//! ephemeral key pair, computes ECDH with the card's public key, and keeps only
//! the ephemeral public key in the vault header. To unlock, the card itself
//! recomputes the same ECDH after its PIN is verified; the shared secret, run
//! through HKDF-SHA-384, opens the card's key slot.
//!
//! Only the bytes on the wire go through the platform: all APDUs are built and
//! parsed here.
use std::future::Future;

use aws_lc_rs::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey};
use aws_lc_rs::rand::SystemRandom;
use zeroize::Zeroizing;

use crate::error::{CoreError, CoreResult};

pub mod openpgp;
#[cfg(not(target_os = "android"))]
pub mod pcsc;
pub mod piv;
#[cfg(test)]
pub mod sim;
pub mod tlv;

/// Sends one APDU and returns the full response including the status word.
pub trait Transport {
    fn transmit(&mut self, apdu: Vec<u8>) -> impl Future<Output = CoreResult<Vec<u8>>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applet {
    OpenPgp,
    Piv,
}

impl Applet {
    pub fn as_str(self) -> &'static str {
        match self {
            Applet::OpenPgp => "openpgp",
            Applet::Piv => "piv",
        }
    }

    pub fn parse(s: &str) -> CoreResult<Self> {
        match s {
            "openpgp" => Ok(Applet::OpenPgp),
            "piv" => Ok(Applet::Piv),
            _ => Err(CoreError::InvalidInput {
                message: "unknown card type".into(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    P256,
    P384,
}

impl Curve {
    pub fn as_str(self) -> &'static str {
        match self {
            Curve::P256 => "p256",
            Curve::P384 => "p384",
        }
    }

    pub fn parse(s: &str) -> CoreResult<Self> {
        match s {
            "p256" => Ok(Curve::P256),
            "p384" => Ok(Curve::P384),
            _ => Err(CoreError::InvalidInput {
                message: "unsupported card curve".into(),
            }),
        }
    }

    fn alg(self) -> &'static agreement::Algorithm {
        match self {
            Curve::P256 => &agreement::ECDH_P256,
            Curve::P384 => &agreement::ECDH_P384,
        }
    }

    /// Length of an uncompressed point (0x04 || X || Y).
    pub fn point_len(self) -> usize {
        match self {
            Curve::P256 => 65,
            Curve::P384 => 97,
        }
    }

    pub const OID_P256: &'static [u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];
    pub const OID_P384: &'static [u8] = &[0x2B, 0x81, 0x04, 0x00, 0x22];

    pub fn oid(self) -> &'static [u8] {
        match self {
            Curve::P256 => Self::OID_P256,
            Curve::P384 => Self::OID_P384,
        }
    }
}

/// The encryption key a card offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardKey {
    pub applet: Applet,
    pub curve: Curve,
    /// Uncompressed public point.
    pub public: Vec<u8>,
    /// Something to show: card maker / serial when the card says.
    pub label: String,
}

/// What a connected card reports before any PIN is entered.
#[derive(Clone, Debug)]
pub struct CardStatus {
    pub key: Option<CardKey>,
    pub applet: Option<Applet>,
    /// PIN tries left, when the card says.
    pub pin_tries: Option<u8>,
    pub label: String,
}

/// Wrong PIN, blocked PIN and other card answers as readable errors.
pub fn status_error(sw: u16, what: &str) -> CoreError {
    let message = match sw {
        0x6983 => {
            "the card PIN is blocked. Unblock it with the card's PUK or admin PIN".to_string()
        }
        s if s & 0xFFF0 == 0x63C0 => match s & 0x0F {
            0 => "wrong PIN. The PIN is now blocked".to_string(),
            1 => "wrong PIN. 1 try left before the PIN is blocked".to_string(),
            n => format!("wrong PIN. {n} tries left"),
        },
        0x6982 => "the card needs its PIN first".into(),
        0x6A82 | 0x6A88 => format!("the card has no {what}"),
        0x6D00 | 0x6E00 => "the card does not support this".into(),
        s => format!("the card refused ({what}, status {s:04X})"),
    };
    CoreError::InvalidInput { message }
}

/// Sends a command, following GET RESPONSE (61xx) and wrong-Le (6Cxx)
/// answers, and chaining data longer than 255 bytes. Returns the body and the
/// final status word.
pub async fn command<T: Transport>(
    t: &mut T,
    cla: u8,
    ins: u8,
    p1: u8,
    p2: u8,
    data: &[u8],
    expect_data: bool,
) -> CoreResult<(Vec<u8>, u16)> {
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(255).collect()
    };
    let mut resp = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let last = i == chunks.len() - 1;
        let mut apdu = vec![if last { cla } else { cla | 0x10 }, ins, p1, p2];
        if !chunk.is_empty() {
            apdu.push(chunk.len() as u8);
            apdu.extend_from_slice(chunk);
        }
        if last && expect_data {
            apdu.push(0x00);
        }
        resp = t.transmit(apdu.clone()).await?;
        if resp.len() < 2 {
            return Err(CoreError::InvalidInput {
                message: "the card stopped answering. Hold it still and try again".into(),
            });
        }
        let sw = sw_of(&resp);
        if !last {
            if sw != 0x9000 {
                return Ok((Vec::new(), sw));
            }
            continue;
        }
        if expect_data && sw & 0xFF00 == 0x6C00 {
            let mut again = apdu;
            *again.last_mut().expect("Le byte") = (sw & 0xFF) as u8;
            resp = t.transmit(again).await?;
            if resp.len() < 2 {
                return Err(CoreError::InvalidInput {
                    message: "the card stopped answering".into(),
                });
            }
        }
    }
    let mut body = resp[..resp.len() - 2].to_vec();
    let mut sw = sw_of(&resp);
    let mut rounds = 0;
    while sw & 0xFF00 == 0x6100 && rounds < 64 {
        rounds += 1;
        let r = t
            .transmit(vec![cla & 0xF0, 0xC0, 0, 0, (sw & 0xFF) as u8])
            .await?;
        if r.len() < 2 {
            return Err(CoreError::InvalidInput {
                message: "the card stopped answering".into(),
            });
        }
        body.extend_from_slice(&r[..r.len() - 2]);
        sw = sw_of(&r);
    }
    Ok((body, sw))
}

fn sw_of(resp: &[u8]) -> u16 {
    let n = resp.len();
    ((resp[n - 2] as u16) << 8) | resp[n - 1] as u16
}

/// Finds which applet the card has and its encryption key. OpenPGP is
/// tried first, then PIV.
pub async fn probe<T: Transport>(t: &mut T) -> CoreResult<CardStatus> {
    if let Some(s) = openpgp::status(t).await? {
        return Ok(s);
    }
    if let Some(s) = piv::status(t).await? {
        return Ok(s);
    }
    Err(CoreError::InvalidInput {
        message: "this card has no OpenPGP or PIV applet Atropos can use".into(),
    })
}

/// Has the card compute ECDH between its private key and `peer` after
/// verifying `pin`. Returns the shared secret (the X coordinate).
pub async fn agree<T: Transport>(
    t: &mut T,
    key: &CardKey,
    pin: &[u8],
    peer: &[u8],
) -> CoreResult<Zeroizing<Vec<u8>>> {
    match key.applet {
        Applet::OpenPgp => openpgp::agree(t, key, pin, peer).await,
        Applet::Piv => piv::agree(t, key, pin, peer).await,
    }
}

/// Makes the ephemeral half for a new card slot: returns (ephemeral public
/// point, shared secret). The ephemeral private key is dropped here.
pub fn enroll_ephemeral(key: &CardKey) -> CoreResult<(Vec<u8>, Zeroizing<Vec<u8>>)> {
    let rng = SystemRandom::new();
    let eph =
        EphemeralPrivateKey::generate(key.curve.alg(), &rng).map_err(|_| CoreError::Crypto)?;
    let public = eph
        .compute_public_key()
        .map_err(|_| CoreError::Crypto)?
        .as_ref()
        .to_vec();
    let peer = UnparsedPublicKey::new(key.curve.alg(), &key.public);
    let shared = agreement::agree_ephemeral(eph, peer, CoreError::Crypto, |s| {
        Ok(Zeroizing::new(s.to_vec()))
    })?;
    Ok((public, shared))
}
