//! Software stand-ins for an OpenPGP card and a PIV card, enough of each to
//! test the protocol code without hardware.
use aws_lc_rs::agreement::{self, PrivateKey};

use super::tlv::{self, find_deep};
use super::{Curve, Transport};
use crate::error::CoreResult;

fn alg(c: Curve) -> &'static agreement::Algorithm {
    match c {
        Curve::P256 => &agreement::ECDH_P256,
        Curve::P384 => &agreement::ECDH_P384,
    }
}

fn ok(mut body: Vec<u8>) -> Vec<u8> {
    body.extend_from_slice(&[0x90, 0x00]);
    body
}

fn sw(s: u16) -> Vec<u8> {
    vec![(s >> 8) as u8, s as u8]
}

pub struct SimCard {
    pub piv: bool,
    pub curve: Curve,
    key: Option<PrivateKey>,
    pub pin: Vec<u8>,
    pub admin_pin: Vec<u8>,
    tries: u8,
    verified: bool,
    selected: bool,
    /// Bytes waiting for GET RESPONSE.
    pending: Vec<u8>,
    chained: Vec<u8>,
    pub apdus: usize,
}

impl SimCard {
    pub fn openpgp(curve: Curve, with_key: bool) -> Self {
        Self::new(false, curve, with_key, b"123456")
    }

    pub fn piv(curve: Curve) -> Self {
        Self::new(true, curve, true, b"123456")
    }

    fn new(piv: bool, curve: Curve, with_key: bool, pin: &[u8]) -> Self {
        Self {
            piv,
            curve,
            key: with_key.then(|| PrivateKey::generate(alg(curve)).unwrap()),
            pin: pin.to_vec(),
            admin_pin: b"12345678".to_vec(),
            tries: 3,
            verified: false,
            selected: false,
            pending: Vec::new(),
            chained: Vec::new(),
            apdus: 0,
        }
    }

    pub fn public(&self) -> Option<Vec<u8>> {
        self.key
            .as_ref()
            .map(|k| k.compute_public_key().unwrap().as_ref().to_vec())
    }

    /// Answers long bodies in 100-byte pieces to exercise GET RESPONSE.
    fn reply(&mut self, body: Vec<u8>) -> Vec<u8> {
        if body.len() <= 100 {
            return ok(body);
        }
        let mut first = body[..100].to_vec();
        self.pending = body[100..].to_vec();
        let left = self.pending.len().min(255) as u8;
        first.extend_from_slice(&[0x61, left]);
        first
    }

    fn verify(&mut self, data: &[u8]) -> Vec<u8> {
        if data.is_empty() {
            return if self.verified {
                sw(0x9000)
            } else {
                sw(0x63C0 | self.tries as u16)
            };
        }
        if self.tries == 0 {
            return sw(0x6983);
        }
        let given: Vec<u8> = data.iter().copied().filter(|&b| b != 0xFF).collect();
        if given == self.pin {
            self.tries = 3;
            self.verified = true;
            sw(0x9000)
        } else {
            self.tries -= 1;
            sw(0x63C0 | self.tries as u16)
        }
    }

    fn ecdh(&self, peer: &[u8]) -> Option<Vec<u8>> {
        let key = self.key.as_ref()?;
        agreement::agree(
            key,
            agreement::UnparsedPublicKey::new(alg(self.curve), peer),
            (),
            |s| Ok(s.to_vec()),
        )
        .ok()
    }

    fn handle(&mut self, apdu: &[u8]) -> Vec<u8> {
        self.apdus += 1;
        let (cla, ins, p1, p2) = (apdu[0], apdu[1], apdu[2], apdu[3]);
        let data: Vec<u8> = if apdu.len() > 5 {
            let lc = apdu[4] as usize;
            apdu[5..5 + lc].to_vec()
        } else {
            Vec::new()
        };
        if cla & 0x10 != 0 {
            self.chained.extend_from_slice(&data);
            return sw(0x9000);
        }
        let data = if self.chained.is_empty() {
            data
        } else {
            let mut all = std::mem::take(&mut self.chained);
            all.extend_from_slice(&data);
            all
        };
        if ins == 0xC0 {
            let n = (p1 as usize).max(apdu.get(4).copied().unwrap_or(0) as usize);
            let _ = n;
            let take = self.pending.len().min(100);
            let mut part: Vec<u8> = self.pending.drain(..take).collect();
            if self.pending.is_empty() {
                part.extend_from_slice(&[0x90, 0x00]);
            } else {
                part.extend_from_slice(&[0x61, self.pending.len().min(255) as u8]);
            }
            return part;
        }
        if ins == 0xA4 {
            let aid = if self.piv {
                super::piv::AID
            } else {
                super::openpgp::AID
            };
            self.selected = data.starts_with(aid);
            self.verified = false;
            return if self.selected {
                sw(0x9000)
            } else {
                sw(0x6A82)
            };
        }
        if !self.selected {
            return sw(0x6D00);
        }
        if self.piv {
            self.handle_piv(ins, p1, p2, &data)
        } else {
            self.handle_openpgp(ins, p1, p2, &data)
        }
    }

    fn handle_openpgp(&mut self, ins: u8, p1: u8, p2: u8, data: &[u8]) -> Vec<u8> {
        match (ins, p1, p2) {
            (0xCA, 0x00, 0x6E) => {
                let mut aid = vec![0xD2, 0x76, 0x00, 0x01, 0x24, 0x01, 0x03, 0x04, 0x00, 0x06];
                aid.extend_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x00, 0x00]);
                let mut attrs = vec![0x12];
                attrs.extend_from_slice(self.curve.oid());
                let mut body = tlv::encode(0x4F, &aid);
                let disc = [
                    tlv::encode(0xC2, &attrs),
                    tlv::encode(0xC4, &[0x00, 0x7F, 0x7F, 0x7F, self.tries, 0x00, 0x03]),
                ]
                .concat();
                body.extend_from_slice(&tlv::encode(0x73, &disc));
                let full = tlv::encode(0x6E, &body);
                self.reply(full)
            }
            (0x47, 0x81, 0x00) => match self.public() {
                Some(p) => {
                    let r = tlv::encode(0x7F49, &tlv::encode(0x86, &p));
                    self.reply(r)
                }
                None => sw(0x6A88),
            },
            (0x20, 0x00, 0x82) => self.verify(data),
            (0x20, 0x00, 0x83) => {
                if data == self.admin_pin.as_slice() {
                    sw(0x9000)
                } else {
                    sw(0x63C2)
                }
            }
            (0xDA, 0x00, 0xC2) => {
                if data[1..] == *Curve::OID_P384 {
                    self.curve = Curve::P384;
                } else if data[1..] == *Curve::OID_P256 {
                    self.curve = Curve::P256;
                }
                sw(0x9000)
            }
            (0x47, 0x80, 0x00) => {
                self.key = Some(PrivateKey::generate(alg(self.curve)).unwrap());
                let p = self.public().unwrap();
                let r = tlv::encode(0x7F49, &tlv::encode(0x86, &p));
                self.reply(r)
            }
            (0x2A, 0x80, 0x86) => {
                if !self.verified {
                    return sw(0x6982);
                }
                let Some(peer) = find_deep(data, 0x86) else {
                    return sw(0x6A80);
                };
                match self.ecdh(peer) {
                    Some(s) => self.reply(s),
                    None => sw(0x6A80),
                }
            }
            _ => sw(0x6D00),
        }
    }

    fn handle_piv(&mut self, ins: u8, p1: u8, p2: u8, data: &[u8]) -> Vec<u8> {
        match (ins, p1, p2) {
            (0xF8, _, _) => ok(vec![0x00, 0xBC, 0x61, 0x4E]),
            (0xF7, 0x00, 0x9D) => {
                let algo = if self.curve == Curve::P384 {
                    0x14
                } else {
                    0x11
                };
                let Some(p) = self.public() else {
                    return sw(0x6A88);
                };
                let mut body = tlv::encode(0x01, &[algo]);
                body.extend_from_slice(&tlv::encode(0x04, &tlv::encode(0x86, &p)));
                self.reply(body)
            }
            (0x20, 0x00, 0x80) => self.verify(data),
            (0x87, _, 0x9D) => {
                if !self.verified {
                    return sw(0x6982);
                }
                let Some(peer) = find_deep(data, 0x85) else {
                    return sw(0x6A80);
                };
                match self.ecdh(peer) {
                    Some(s) => {
                        let r = tlv::encode(0x7C, &tlv::encode(0x82, &s));
                        self.reply(r)
                    }
                    None => sw(0x6A80),
                }
            }
            _ => sw(0x6D00),
        }
    }
}

impl Transport for SimCard {
    async fn transmit(&mut self, apdu: Vec<u8>) -> CoreResult<Vec<u8>> {
        Ok(self.handle(&apdu))
    }
}

/// Runs a future to completion on this thread (the simulator never waits).
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::task::{Context, Poll, Waker};
    let mut cx = Context::from_waker(Waker::noop());
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    fn round_trip(mut card: SimCard) {
        let st = block_on(probe(&mut card)).unwrap();
        let key = st.key.expect("card key");
        assert_eq!(Some(key.public.clone()), card.public());
        let (eph, shared) = enroll_ephemeral(&key).unwrap();
        let on_card = block_on(agree(&mut card, &key, b"123456", &eph)).unwrap();
        assert_eq!(*on_card, *shared);
        // Wrong PIN counts down and says how many tries are left.
        let e = block_on(agree(&mut card, &key, b"000000", &eph)).unwrap_err();
        assert!(format!("{e:?}").contains("2 tries left"), "{e:?}");
    }

    #[test]
    fn openpgp_p384_and_p256() {
        round_trip(SimCard::openpgp(Curve::P384, true));
        round_trip(SimCard::openpgp(Curve::P256, true));
    }

    #[test]
    fn piv_p384_and_p256() {
        round_trip(SimCard::piv(Curve::P384));
        round_trip(SimCard::piv(Curve::P256));
    }

    #[test]
    fn openpgp_without_key_then_generate() {
        let mut card = SimCard::openpgp(Curve::P256, false);
        let st = block_on(probe(&mut card)).unwrap();
        assert!(st.key.is_none());
        assert_eq!(st.pin_tries, Some(3));
        assert!(block_on(openpgp::generate(&mut card, b"wrong", Curve::P384)).is_err());
        let key = block_on(openpgp::generate(&mut card, b"12345678", Curve::P384)).unwrap();
        assert_eq!(key.curve, Curve::P384);
        let st = block_on(probe(&mut card)).unwrap();
        assert_eq!(st.key.unwrap().public, key.public);
    }

    #[test]
    fn long_commands_are_chained() {
        let mut card = SimCard::openpgp(Curve::P384, true);
        block_on(command(&mut card, 0, 0xA4, 4, 0, openpgp::AID, true)).unwrap();
        let before = card.apdus;
        let data = vec![1u8; 600];
        let (_, sw) = block_on(command(&mut card, 0, 0xDA, 0, 0x99, &data, false)).unwrap();
        assert_eq!(sw, 0x6D00);
        assert_eq!(card.apdus - before, 3);
    }
}
