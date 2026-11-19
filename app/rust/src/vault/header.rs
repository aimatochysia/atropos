//! `vault.hdr`: the only clear-text file in a vault.
//!
//! It holds the vault's public key and its key slots. Each slot wraps the
//! same secret bundle (ML-KEM-1024 secret key, P-384 secret key and a 256-bit
//! root key) under AES-256-GCM with a key derived from one unlock method.
//! Each slot's associated data binds the vault id, the slot id and kind, and a
//! hash of the public key, so a swapped public key or a slot moved from
//! another vault fails to unwrap. A whole-header HMAC keyed from the root key
//! catches any other edit once the vault is unlocked.
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::aead::AeadKey;
use crate::crypto::hybrid::{HybridPublicKey, HybridSecretKey, MLKEM_SECRET_LEN, P384_SECRET_LEN};
use crate::crypto::kdf::derive_key;
use crate::crypto::password::{self, Argon2Cost};
use crate::crypto::{hmac_sha384, hmac_sha384_verify, random_array, sha384};
use crate::error::{CoreError, CoreResult};

pub const FORMAT: &str = "atropos-vault";
pub const VERSION: u32 = 1;
pub const SUITE: &str = "ML-KEM-1024+ECDH-P384/HKDF-SHA384/AES-256-GCM/Argon2id";
pub const ROOT_LEN: usize = 32;
const BUNDLE_LEN: usize = MLKEM_SECRET_LEN + P384_SECRET_LEN + ROOT_LEN;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    Password,
    Recovery,
    /// A secret held by one device's hardware keystore behind biometrics.
    Device,
}

/// How long a device (biometric) slot works after the last unlock with the
/// password or recovery phrase on that device.
pub const DEVICE_TTL_SECS: i64 = 24 * 3600;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CostJson {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl From<Argon2Cost> for CostJson {
    fn from(c: Argon2Cost) -> Self {
        Self {
            memory_kib: c.memory_kib,
            iterations: c.iterations,
            parallelism: c.parallelism,
        }
    }
}

impl From<CostJson> for Argon2Cost {
    fn from(c: CostJson) -> Self {
        Self {
            memory_kib: c.memory_kib,
            iterations: c.iterations,
            parallelism: c.parallelism,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Slot {
    #[serde(with = "super::b64")]
    pub id: Vec<u8>,
    pub kind: SlotKind,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub argon2: Option<CostJson>,
    #[serde(with = "super::b64")]
    pub salt: Vec<u8>,
    #[serde(with = "super::b64")]
    pub nonce: Vec<u8>,
    #[serde(with = "super::b64")]
    pub wrapped: Vec<u8>,
    /// Device slots: which installation holds the secret.
    #[serde(
        with = "super::b64::opt",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub device: Option<Vec<u8>>,
    /// Device slots: a name to show, such as the phone model.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
    /// Device slots: Unix time after which the slot refuses to open. Not part
    /// of the slot's associated data (it changes without re-wrapping) but
    /// covered by the header MAC, which is checked before the slot is used.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub expires: Option<i64>,
    /// Device slots: when the expiry was last renewed, to spot a clock that
    /// was turned back.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub renewed: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PublicKeyJson {
    #[serde(with = "super::b64")]
    pub mlkem: Vec<u8>,
    #[serde(with = "super::b64")]
    pub p384: Vec<u8>,
}

/// Everything in the header except its MAC, in a fixed field order so the
/// MAC input is reproducible.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HeaderBody {
    pub format: String,
    pub version: u32,
    pub suite: String,
    #[serde(with = "super::b64")]
    pub vault_id: Vec<u8>,
    pub created: i64,
    pub public_key: PublicKeyJson,
    pub slots: Vec<Slot>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HeaderFile {
    #[serde(flatten)]
    pub body: HeaderBody,
    #[serde(with = "super::b64")]
    pub mac: Vec<u8>,
}

/// The unwrapped secrets of an unlocked vault. Wiped on drop.
pub struct VaultSecrets {
    pub keypair: HybridSecretKey,
    pub root: Zeroizing<[u8; ROOT_LEN]>,
}

impl VaultSecrets {
    pub fn generate() -> CoreResult<Self> {
        Ok(Self {
            keypair: HybridSecretKey::generate()?,
            root: Zeroizing::new(random_array::<ROOT_LEN>()?),
        })
    }

    fn bundle(&self) -> Zeroizing<Vec<u8>> {
        let (m, p) = self.keypair.secret_parts();
        let mut out = Zeroizing::new(Vec::with_capacity(BUNDLE_LEN));
        out.extend_from_slice(&m);
        out.extend_from_slice(&p);
        out.extend_from_slice(self.root.as_ref());
        out
    }

    fn from_bundle(bundle: &[u8], public: HybridPublicKey) -> CoreResult<Self> {
        if bundle.len() != BUNDLE_LEN {
            return Err(CoreError::DecryptFailed);
        }
        let (m, rest) = bundle.split_at(MLKEM_SECRET_LEN);
        let (p, r) = rest.split_at(P384_SECRET_LEN);
        let mut root = Zeroizing::new([0u8; ROOT_LEN]);
        root.copy_from_slice(r);
        Ok(Self {
            keypair: HybridSecretKey::from_parts(m, p, public)?,
            root,
        })
    }

    /// A purpose-specific key derived from the root key.
    pub fn subkey(&self, label: &[u8], vault_id: &[u8]) -> CoreResult<Zeroizing<[u8; 32]>> {
        derive_key(b"atropos/vault/v1", self.root.as_ref(), &[label, vault_id])
    }
}

/// How a slot is opened.
pub enum Unlock<'a> {
    Password(&'a [u8]),
    Recovery(&'a [u8; 32]),
    Device {
        device: &'a [u8],
        secret: &'a [u8; 32],
    },
}

impl Unlock<'_> {
    /// Password and recovery phrase prove the person knows the vault's
    /// secrets; a device slot only proves they hold this device.
    pub fn is_strong(&self) -> bool {
        !matches!(self, Unlock::Device { .. })
    }
}

fn slot_aad(body: &HeaderBody, slot: &Slot) -> Vec<u8> {
    let pk_hash = sha384(&[&body.public_key.mlkem, &body.public_key.p384]);
    let mut aad = b"atropos/slot/v1".to_vec();
    aad.extend_from_slice(&body.vault_id);
    aad.extend_from_slice(&slot.id);
    aad.push(match slot.kind {
        SlotKind::Password => 1,
        SlotKind::Recovery => 2,
        SlotKind::Device => 3,
    });
    aad.extend_from_slice(&pk_hash);
    if let Some(d) = &slot.device {
        aad.extend_from_slice(b"device");
        aad.extend_from_slice(d);
    }
    aad
}

fn slot_key(slot: &Slot, how: &Unlock) -> CoreResult<Option<Zeroizing<[u8; 32]>>> {
    let (kind, salt, cost) = (slot.kind, slot.salt.as_slice(), slot.argon2);
    match (kind, how) {
        (SlotKind::Password, Unlock::Password(pw)) => {
            let salt: [u8; password::SALT_LEN] =
                salt.try_into().map_err(|_| CoreError::DecryptFailed)?;
            let cost = cost.ok_or(CoreError::DecryptFailed)?;
            Ok(Some(password::derive(pw, &salt, cost.into())?))
        }
        (SlotKind::Recovery, Unlock::Recovery(k)) => Ok(Some(derive_key(
            salt,
            k.as_ref(),
            &[b"atropos/slot/recovery/v1"],
        )?)),
        (SlotKind::Device, Unlock::Device { device, secret })
            if slot.device.as_deref() == Some(*device) =>
        {
            Ok(Some(derive_key(
                salt,
                secret.as_ref(),
                &[b"atropos/slot/device/v1"],
            )?))
        }
        _ => Ok(None),
    }
}

impl HeaderBody {
    pub fn new(secrets: &VaultSecrets, created: i64) -> CoreResult<Self> {
        let pk = secrets.keypair.public_key();
        Ok(Self {
            format: FORMAT.into(),
            version: VERSION,
            suite: SUITE.into(),
            vault_id: random_array::<16>()?.to_vec(),
            created,
            public_key: PublicKeyJson {
                mlkem: pk.mlkem.clone(),
                p384: pk.p384.clone(),
            },
            slots: Vec::new(),
        })
    }

    pub fn public_key(&self) -> CoreResult<HybridPublicKey> {
        HybridPublicKey::from_parts(&self.public_key.mlkem, &self.public_key.p384)
    }

    /// Adds a slot that `how` opens. A password slot uses `cost`.
    pub fn add_slot(
        &mut self,
        secrets: &VaultSecrets,
        how: &Unlock,
        cost: Argon2Cost,
    ) -> CoreResult<()> {
        let (kind, salt, argon2, device) = match how {
            Unlock::Password(_) => (
                SlotKind::Password,
                random_array::<{ password::SALT_LEN }>()?.to_vec(),
                Some(CostJson::from(cost)),
                None,
            ),
            Unlock::Recovery(_) => (
                SlotKind::Recovery,
                random_array::<32>()?.to_vec(),
                None,
                None,
            ),
            Unlock::Device { device, .. } => (
                SlotKind::Device,
                random_array::<32>()?.to_vec(),
                None,
                Some(device.to_vec()),
            ),
        };
        let mut slot = Slot {
            id: random_array::<8>()?.to_vec(),
            kind,
            argon2,
            salt,
            nonce: Vec::new(),
            wrapped: Vec::new(),
            device,
            label: None,
            expires: None,
            renewed: None,
        };
        let key = slot_key(&slot, how)?.ok_or(CoreError::Crypto)?;
        let nonce = random_array::<12>()?;
        let aad = slot_aad(self, &slot);
        slot.wrapped = AeadKey::new(&key)?.seal(&nonce, &aad, &secrets.bundle())?;
        slot.nonce = nonce.to_vec();
        self.slots.push(slot);
        Ok(())
    }

    /// Removes every slot of `kind`.
    pub fn remove_slots(&mut self, kind: SlotKind) {
        self.slots.retain(|s| s.kind != kind);
    }

    /// Removes the device slot of `device`, if any.
    pub fn remove_device(&mut self, device: &[u8]) {
        self.slots
            .retain(|s| !(s.kind == SlotKind::Device && s.device.as_deref() == Some(device)));
    }

    pub fn device_slot_mut(&mut self, device: &[u8]) -> Option<&mut Slot> {
        self.slots
            .iter_mut()
            .find(|s| s.kind == SlotKind::Device && s.device.as_deref() == Some(device))
    }

    pub fn device_slot(&self, device: &[u8]) -> Option<&Slot> {
        self.slots
            .iter()
            .find(|s| s.kind == SlotKind::Device && s.device.as_deref() == Some(device))
    }

    /// Tries each slot that matches `how`. Returns the secrets and the index of
    /// the slot that opened.
    pub fn unlock(&self, how: &Unlock) -> CoreResult<(VaultSecrets, usize)> {
        let public = self.public_key().map_err(|_| CoreError::DecryptFailed)?;
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(cost) = slot.argon2 {
                Argon2Cost::from(cost).validate()?;
            }
            let Some(key) = slot_key(slot, how)? else {
                continue;
            };
            let nonce: [u8; 12] = slot
                .nonce
                .as_slice()
                .try_into()
                .map_err(|_| CoreError::DecryptFailed)?;
            let aad = slot_aad(self, slot);
            if let Ok(mut bundle) = AeadKey::new(&key)?.open(&nonce, &aad, &slot.wrapped) {
                let secrets = VaultSecrets::from_bundle(&bundle, public)?;
                bundle.zeroize();
                return Ok((secrets, i));
            }
        }
        Err(CoreError::DecryptFailed)
    }

    /// Re-wraps slot `index` under a fresh nonce (same slot key), so the stored
    /// blob changes on every unlock.
    pub fn reseal(&mut self, index: usize, secrets: &VaultSecrets, how: &Unlock) -> CoreResult<()> {
        let slot = self.slots.get(index).ok_or(CoreError::Crypto)?.clone();
        let key = slot_key(&slot, how)?.ok_or(CoreError::Crypto)?;
        let nonce = random_array::<12>()?;
        let aad = slot_aad(self, &slot);
        let wrapped = AeadKey::new(&key)?.seal(&nonce, &aad, &secrets.bundle())?;
        let s = &mut self.slots[index];
        s.nonce = nonce.to_vec();
        s.wrapped = wrapped;
        Ok(())
    }

    fn mac_input(&self) -> CoreResult<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }

    pub fn seal(self, secrets: &VaultSecrets) -> CoreResult<HeaderFile> {
        let key = secrets.subkey(b"header-mac", &self.vault_id)?;
        let mac = hmac_sha384(key.as_ref(), &[&self.mac_input()?]);
        Ok(HeaderFile { body: self, mac })
    }
}

impl HeaderFile {
    pub fn parse(bytes: &[u8]) -> CoreResult<Self> {
        let h: HeaderFile = serde_json::from_slice(bytes).map_err(|_| CoreError::InvalidInput {
            message: "this folder is not an Atropos vault, or its header is damaged".into(),
        })?;
        if h.body.format != FORMAT {
            return Err(CoreError::InvalidInput {
                message: "this folder is not an Atropos vault".into(),
            });
        }
        if h.body.version != VERSION || h.body.suite != SUITE {
            return Err(CoreError::InvalidInput {
                message: "this vault was made by a newer version of Atropos; update the app".into(),
            });
        }
        if h.body.vault_id.len() != 16 {
            return Err(CoreError::InvalidInput {
                message: "the vault header is damaged".into(),
            });
        }
        Ok(h)
    }

    /// Checks the header MAC once the secrets are known.
    pub fn verify(&self, secrets: &VaultSecrets) -> CoreResult<()> {
        let key = secrets.subkey(b"header-mac", &self.body.vault_id)?;
        if hmac_sha384_verify(key.as_ref(), &[&self.body.mac_input()?], &self.mac) {
            Ok(())
        } else {
            Err(CoreError::InvalidInput {
                message: "the vault header was changed outside Atropos".into(),
            })
        }
    }

    pub fn to_bytes(&self) -> CoreResult<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: Argon2Cost = Argon2Cost::MINIMUM;

    fn new_header() -> (HeaderFile, VaultSecrets, [u8; 32]) {
        let secrets = VaultSecrets::generate().unwrap();
        let mut body = HeaderBody::new(&secrets, 0).unwrap();
        body.add_slot(&secrets, &Unlock::Password(b"correct horse"), FAST)
            .unwrap();
        let rk = [42u8; 32];
        body.add_slot(&secrets, &Unlock::Recovery(&rk), FAST)
            .unwrap();
        (body.seal(&secrets).unwrap(), secrets, rk)
    }

    #[test]
    fn unlock_with_each_slot() {
        let (h, secrets, rk) = new_header();
        let h = HeaderFile::parse(&h.to_bytes().unwrap()).unwrap();
        let (a, ia) = h.body.unlock(&Unlock::Password(b"correct horse")).unwrap();
        assert_eq!(ia, 0);
        assert_eq!(*a.root, *secrets.root);
        h.verify(&a).unwrap();
        let (b, ib) = h.body.unlock(&Unlock::Recovery(&rk)).unwrap();
        assert_eq!(ib, 1);
        assert_eq!(*b.root, *secrets.root);
        assert!(h.body.unlock(&Unlock::Password(b"wrong")).is_err());
        assert!(h.body.unlock(&Unlock::Recovery(&[0u8; 32])).is_err());
    }

    #[test]
    fn swapped_public_key_breaks_unlock() {
        let (mut h, _, _) = new_header();
        let other = VaultSecrets::generate().unwrap();
        h.body.public_key.mlkem = other.keypair.public_key().mlkem.clone();
        assert!(h.body.unlock(&Unlock::Password(b"correct horse")).is_err());
    }

    #[test]
    fn edited_header_fails_mac() {
        let (mut h, secrets, _) = new_header();
        h.body.created = 99;
        assert!(h.verify(&secrets).is_err());
    }

    #[test]
    fn reseal_changes_bytes_but_still_opens() {
        let (h, secrets, _) = new_header();
        let mut body = h.body.clone();
        body.reseal(0, &secrets, &Unlock::Password(b"correct horse"))
            .unwrap();
        assert_ne!(body.slots[0].wrapped, h.body.slots[0].wrapped);
        let sealed = body.seal(&secrets).unwrap();
        sealed
            .body
            .unlock(&Unlock::Password(b"correct horse"))
            .unwrap();
        sealed.verify(&secrets).unwrap();
    }
}
