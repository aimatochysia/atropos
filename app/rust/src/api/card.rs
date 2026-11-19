//! Smartcards (OpenPGP and PIV) as vault keys.
//!
//! Every card call takes `transmit`: a function that sends one APDU to the
//! card and returns the answer (an empty answer means the card is gone). On
//! desktops it wraps a [CardReader] (PC/SC); on Android it goes to the NFC
//! tag Kotlin is holding. All APDUs are built and checked in Rust.
use std::path::Path;

use flutter_rust_bridge::{frb, DartFnFuture};

use super::secure_input::SecretInput;
use super::vault::{session_of, strong_session, VaultSession};
use crate::card::{self, Applet, CardKey, Curve, Transport};
use crate::error::{CoreError, CoreResult};
use crate::vault::header::{CardSlotJson, Unlock};
use crate::vault::Vault;

struct DartTransport<F>(F);

impl<F> Transport for DartTransport<F>
where
    F: Fn(Vec<u8>) -> DartFnFuture<Vec<u8>>,
{
    fn transmit(
        &mut self,
        apdu: Vec<u8>,
    ) -> impl std::future::Future<Output = CoreResult<Vec<u8>>> {
        let fut = (self.0)(apdu);
        async move { Ok(fut.await) }
    }
}

pub struct CardInfo {
    /// "openpgp" or "piv".
    pub applet: String,
    pub label: String,
    /// The card has an encryption key Atropos can use.
    pub has_key: bool,
    /// "p384" or "p256" when it has a key.
    pub curve: Option<String>,
    pub pin_tries: Option<u8>,
}

pub struct CardSlotInfo {
    /// Hex slot id, for `remove_card`.
    pub id: String,
    pub label: String,
    pub applet: String,
    pub added: i64,
}

fn info(st: &card::CardStatus) -> CardInfo {
    CardInfo {
        applet: st
            .applet
            .map(|a| a.as_str().to_string())
            .unwrap_or_default(),
        label: st.label.clone(),
        has_key: st.key.is_some(),
        curve: st.key.as_ref().map(|k| k.curve.as_str().to_string()),
        pin_tries: st.pin_tries,
    }
}

/// What card is there, before any PIN.
pub async fn card_status(
    transmit: impl Fn(Vec<u8>) -> DartFnFuture<Vec<u8>>,
) -> CoreResult<CardInfo> {
    let mut t = DartTransport(transmit);
    Ok(info(&card::probe(&mut t).await?))
}

/// Makes a new P-384 encryption key on an OpenPGP card with its admin PIN
/// (factory default 12345678). Replaces the card's current decryption key.
pub async fn card_generate_key(
    admin_pin: &SecretInput,
    transmit: impl Fn(Vec<u8>) -> DartFnFuture<Vec<u8>>,
) -> CoreResult<CardInfo> {
    let pin = admin_pin.bytes();
    let mut t = DartTransport(transmit);
    let st = card::probe(&mut t).await?;
    if st.applet != Some(Applet::OpenPgp) {
        return Err(CoreError::InvalidInput {
            message: "Atropos can only set up OpenPGP cards. Make a P-384 key in slot 9D of a \
                      PIV card with your card's own tool (for example YubiKey Manager)"
                .into(),
        });
    }
    card::openpgp::generate(&mut t, &pin, Curve::P384).await?;
    Ok(info(&card::probe(&mut t).await?))
}

fn key_of(c: &CardSlotJson, label: &str) -> CoreResult<CardKey> {
    Ok(CardKey {
        applet: Applet::parse(&c.applet)?,
        curve: Curve::parse(&c.curve)?,
        public: c.public.clone(),
        label: label.to_string(),
    })
}

/// Unlocks with a card and its PIN. Counts as a strong unlock, so it also
/// renews fingerprint unlock on this device.
pub async fn unlock_with_card(
    path: String,
    pin: &SecretInput,
    device_id: Option<String>,
    transmit: impl Fn(Vec<u8>) -> DartFnFuture<Vec<u8>>,
) -> CoreResult<VaultSession> {
    let pin = pin.bytes();
    let slots = Vault::card_slots(Path::new(&path))?;
    if slots.is_empty() {
        return Err(CoreError::InvalidInput {
            message: "no security card has been added to this vault".into(),
        });
    }
    let mut t = DartTransport(transmit);
    let st = card::probe(&mut t).await?;
    let present = st.key.ok_or(CoreError::InvalidInput {
        message: "this card has no encryption key".into(),
    })?;
    let slot =
        slots
            .iter()
            .find(|s| s.public == present.public)
            .ok_or(CoreError::InvalidInput {
                message: "this card was not added to this vault".into(),
            })?;
    let key = key_of(slot, &present.label)?;
    let shared = card::agree(&mut t, &key, &pin, &slot.ephemeral).await?;
    let v = Vault::unlock(
        Path::new(&path),
        &Unlock::Card {
            card: slot,
            shared: &shared,
        },
    )
    .map_err(|e| match e {
        CoreError::DecryptFailed => CoreError::InvalidInput {
            message: "the card's answer did not open the vault".into(),
        },
        e => e,
    })?;
    Ok(strong_session(v, device_id))
}

/// How many cards a locked vault accepts (to show the card button).
pub fn vault_card_count(path: String) -> CoreResult<u32> {
    Ok(Vault::card_slots(Path::new(&path))?.len() as u32)
}

impl VaultSession {
    /// Adds the card that `transmit` reaches. Checks the PIN and that the card
    /// really computes the expected secret before the slot is saved.
    pub async fn add_card(
        &self,
        pin: &SecretInput,
        name: String,
        transmit: impl Fn(Vec<u8>) -> DartFnFuture<Vec<u8>>,
    ) -> CoreResult<String> {
        let pin = pin.bytes();
        let mut t = DartTransport(transmit);
        let st = card::probe(&mut t).await?;
        let key = st.key.ok_or(CoreError::InvalidInput {
            message: "this card has no encryption key yet. Set one up first".into(),
        })?;
        let (ephemeral, shared) = card::enroll_ephemeral(&key)?;
        let on_card = card::agree(&mut t, &key, &pin, &ephemeral).await?;
        if !bool::from(subtle::ConstantTimeEq::ct_eq(
            on_card.as_slice(),
            shared.as_slice(),
        )) {
            return Err(CoreError::InvalidInput {
                message: "the card answered with the wrong secret".into(),
            });
        }
        let slot = CardSlotJson {
            applet: key.applet.as_str().into(),
            curve: key.curve.as_str().into(),
            public: key.public.clone(),
            ephemeral,
        };
        let label = if name.trim().is_empty() {
            key.label.clone()
        } else {
            name.trim().to_string()
        };
        session_of(self, |v| v.add_card(slot, &shared, &label))?;
        Ok(label)
    }

    pub fn cards(&self) -> CoreResult<Vec<CardSlotInfo>> {
        session_of(self, |v| {
            Ok(v.cards()
                .into_iter()
                .map(|(id, label, added, c)| CardSlotInfo {
                    id: hex::encode(id),
                    label,
                    applet: c.applet,
                    added,
                })
                .collect())
        })
    }

    pub fn remove_card(&self, id: String) -> CoreResult<()> {
        let id = hex::decode(id).map_err(|_| CoreError::InvalidInput {
            message: "bad card id".into(),
        })?;
        session_of(self, |v| v.remove_card(&id))
    }
}

/// A PC/SC smartcard reader connection (desktop only).
#[frb(opaque)]
pub struct CardReader {
    #[cfg(not(target_os = "android"))]
    conn: card::pcsc::Connection,
}

/// Names of connected PC/SC readers. Empty on Android (cards use NFC).
pub fn card_readers() -> CoreResult<Vec<String>> {
    #[cfg(not(target_os = "android"))]
    {
        card::pcsc::readers()
    }
    #[cfg(target_os = "android")]
    {
        Ok(Vec::new())
    }
}

/// Connects to `reader`, or the first reader holding a card.
pub fn open_card_reader(reader: Option<String>) -> CoreResult<CardReader> {
    #[cfg(not(target_os = "android"))]
    {
        Ok(CardReader {
            conn: card::pcsc::Connection::open(reader.as_deref())?,
        })
    }
    #[cfg(target_os = "android")]
    {
        let _ = reader;
        Err(CoreError::InvalidInput {
            message: "use NFC on Android".into(),
        })
    }
}

impl CardReader {
    #[frb(sync, getter)]
    pub fn name(&self) -> String {
        #[cfg(not(target_os = "android"))]
        {
            self.conn.reader.clone()
        }
        #[cfg(target_os = "android")]
        {
            String::new()
        }
    }

    /// Sends one APDU. Errors come back as an empty answer so a card pulled
    /// out mid-way reads as "the card stopped answering".
    pub fn transmit(&self, apdu: Vec<u8>) -> Vec<u8> {
        #[cfg(not(target_os = "android"))]
        {
            self.conn.transmit(&apdu).unwrap_or_default()
        }
        #[cfg(target_os = "android")]
        {
            let _ = apdu;
            Vec::new()
        }
    }
}
