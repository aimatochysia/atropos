//! Contact (and desktop NFC) readers through PC/SC: winscard on Windows, the
//! PCSC framework on macOS, pcsc-lite on Linux.
use std::ffi::CString;
use std::sync::Mutex;

use crate::error::{CoreError, CoreResult};

fn err(e: pcsc::Error) -> CoreError {
    let message = match e {
        pcsc::Error::NoService | pcsc::Error::ServiceStopped => {
            "the smartcard service is not running. On Linux install and start pcscd".into()
        }
        pcsc::Error::NoReadersAvailable => "no smartcard reader is connected".into(),
        pcsc::Error::NoSmartcard | pcsc::Error::RemovedCard => {
            "no card in the reader. Insert the card and try again".into()
        }
        pcsc::Error::SharingViolation => {
            "another app is using the card. Close it (for example GnuPG's scdaemon) and try again"
                .into()
        }
        e => format!("smartcard reader error: {e}"),
    };
    CoreError::InvalidInput { message }
}

pub fn readers() -> CoreResult<Vec<String>> {
    let ctx = pcsc::Context::establish(pcsc::Scope::User).map_err(err)?;
    let mut buf = vec![0u8; ctx.list_readers_len().map_err(err)?];
    let list = match ctx.list_readers(&mut buf) {
        Ok(l) => l,
        Err(pcsc::Error::NoReadersAvailable) => return Ok(Vec::new()),
        Err(e) => return Err(err(e)),
    };
    Ok(list.map(|r| r.to_string_lossy().into_owned()).collect())
}

/// An open connection to the card in one reader.
pub struct Connection {
    card: Mutex<pcsc::Card>,
    pub reader: String,
}

impl Connection {
    /// Connects to `reader`, or to the first reader that holds a card.
    pub fn open(reader: Option<&str>) -> CoreResult<Self> {
        let ctx = pcsc::Context::establish(pcsc::Scope::User).map_err(err)?;
        let names = match reader {
            Some(r) => vec![r.to_string()],
            None => readers()?,
        };
        if names.is_empty() {
            return Err(err(pcsc::Error::NoReadersAvailable));
        }
        let mut last = pcsc::Error::NoSmartcard;
        for name in names {
            let c = CString::new(name.clone()).map_err(|_| CoreError::InvalidInput {
                message: "bad reader name".into(),
            })?;
            match ctx.connect(&c, pcsc::ShareMode::Shared, pcsc::Protocols::ANY) {
                Ok(card) => {
                    return Ok(Self {
                        card: Mutex::new(card),
                        reader: name,
                    })
                }
                Err(e) => last = e,
            }
        }
        Err(err(last))
    }

    pub fn transmit(&self, apdu: &[u8]) -> CoreResult<Vec<u8>> {
        let card = self.card.lock().unwrap_or_else(|e| e.into_inner());
        let mut buf = vec![0u8; pcsc::MAX_BUFFER_SIZE_EXTENDED];
        let r = card.transmit(apdu, &mut buf).map_err(err)?;
        Ok(r.to_vec())
    }
}
