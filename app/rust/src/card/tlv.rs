//! BER-TLV as used by ISO 7816 cards (one- and two-byte tags, short and long
//! lengths).

/// One parsed element: tag (1 or 2 bytes packed big-endian) and value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub tag: u16,
    pub value: &'a [u8],
}

/// Parses consecutive TLVs; stops at the first malformed one.
pub fn parse(mut data: &[u8]) -> Vec<Tlv<'_>> {
    let mut out = Vec::new();
    while !data.is_empty() {
        // Padding bytes between objects.
        if data[0] == 0x00 || data[0] == 0xFF {
            data = &data[1..];
            continue;
        }
        let mut tag = data[0] as u16;
        let mut i = 1;
        if data[0] & 0x1F == 0x1F {
            let Some(&b) = data.get(1) else { break };
            tag = (tag << 8) | b as u16;
            i = 2;
        }
        let Some(&l0) = data.get(i) else { break };
        i += 1;
        let len = match l0 {
            0x00..=0x7F => l0 as usize,
            0x81 => {
                let Some(&b) = data.get(i) else { break };
                i += 1;
                b as usize
            }
            0x82 => {
                let (Some(&a), Some(&b)) = (data.get(i), data.get(i + 1)) else {
                    break;
                };
                i += 2;
                ((a as usize) << 8) | b as usize
            }
            _ => break,
        };
        let Some(value) = data.get(i..i + len) else {
            break;
        };
        out.push(Tlv { tag, value });
        data = &data[i + len..];
    }
    out
}

/// Finds `tag` at the top level of `data`.
pub fn find(data: &[u8], tag: u16) -> Option<&[u8]> {
    parse(data)
        .into_iter()
        .find(|t| t.tag == tag)
        .map(|t| t.value)
}

/// Finds `tag` anywhere, descending into constructed elements.
pub fn find_deep(data: &[u8], tag: u16) -> Option<&[u8]> {
    for t in parse(data) {
        if t.tag == tag {
            return Some(t.value);
        }
        let first = if t.tag > 0xFF {
            (t.tag >> 8) as u8
        } else {
            t.tag as u8
        };
        if first & 0x20 != 0 {
            if let Some(v) = find_deep(t.value, tag) {
                return Some(v);
            }
        }
    }
    None
}

/// Encodes one TLV.
pub fn encode(tag: u16, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() + 5);
    if tag > 0xFF {
        out.push((tag >> 8) as u8);
    }
    out.push(tag as u8);
    match value.len() {
        n @ 0..=0x7F => out.push(n as u8),
        n @ 0x80..=0xFF => {
            out.push(0x81);
            out.push(n as u8);
        }
        n => {
            out.push(0x82);
            out.push((n >> 8) as u8);
            out.push(n as u8);
        }
    }
    out.extend_from_slice(value);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_nesting() {
        let inner = encode(0x86, &[4u8; 97]);
        let outer = encode(0x7F49, &inner);
        let wrapped = encode(0xA6, &outer);
        assert_eq!(find_deep(&wrapped, 0x86).unwrap(), &[4u8; 97]);
        assert_eq!(find(&wrapped, 0xA6).unwrap(), outer.as_slice());
        let long = encode(0x53, &[1u8; 300]);
        assert_eq!(find(&long, 0x53).unwrap().len(), 300);
        assert!(find(&[0x53, 0x05, 1], 0x53).is_none());
    }
}
