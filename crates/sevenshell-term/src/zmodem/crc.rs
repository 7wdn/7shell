//! The two checksums ZMODEM puts on the wire.
//!
//! The 16-bit CRC is the XMODEM one (polynomial 0x1021, seeded with zero). The 32-bit CRC is the
//! ordinary reflected CRC-32 (polynomial 0xedb88320, seeded with all ones and inverted at the end),
//! not the all-zero-seeded variant some documents imply.
//!
//! The byte order differs between the two and is not a detail that can be guessed: the 16-bit
//! checksum travels high byte first, the 32-bit one low byte first. Both orders are pinned by
//! tests against captured header bytes, because a wrong order is invisible until a real `rz`
//! rejects every block.

const CRC16_POLYNOMIAL: u16 = 0x1021;
const CRC32_POLYNOMIAL: u32 = 0xedb8_8320;

const fn crc16_table() -> [u16; 256] {
    let mut table = [0u16; 256];
    let mut index = 0usize;
    while index < 256 {
        let mut value = (index as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 0x8000 != 0 {
                (value << 1) ^ CRC16_POLYNOMIAL
            } else {
                value << 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

const fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 != 0 {
                (value >> 1) ^ CRC32_POLYNOMIAL
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

static CRC16_TABLE: [u16; 256] = crc16_table();
static CRC32_TABLE: [u32; 256] = crc32_table();

/// CRC-16/XMODEM over `data`, seeded with zero.
pub fn crc16(data: &[u8]) -> u16 {
    crc16_update(0, data)
}

/// Continues a CRC-16 over another slice. Data subpackets chain the payload and the frame end
/// character here so the frame end needs no copy.
pub(crate) fn crc16_update(crc: u16, data: &[u8]) -> u16 {
    let mut crc = crc;
    for &byte in data {
        crc = (crc << 8) ^ CRC16_TABLE[usize::from((crc >> 8) as u8 ^ byte)];
    }
    crc
}

/// CRC-32 over `data`, seeded with all ones and inverted, as the captured streams check out.
pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(0xffff_ffff, data) ^ 0xffff_ffff
}

/// Continues a CRC-32 over another slice; the caller owns the seed and the final inversion.
pub(crate) fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut crc = crc;
    for &byte in data {
        crc = (crc >> 8) ^ CRC32_TABLE[((crc ^ u32::from(byte)) & 0xff) as usize];
    }
    crc
}

/// The two bytes a CRC-16 occupies on the wire: high byte first.
pub(crate) const fn crc16_wire_bytes(crc: u16) -> [u8; 2] {
    crc.to_be_bytes()
}

/// The four bytes a CRC-32 occupies on the wire: low byte first.
pub(crate) const fn crc32_wire_bytes(crc: u32) -> [u8; 4] {
    crc.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc16_matches_the_published_xmodem_check_vector() {
        assert_eq!(crc16(b"123456789"), 0x31c3);
    }

    #[test]
    fn crc32_matches_the_published_crc32_check_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn crc16_of_a_captured_zfile_header_is_0x9927() {
        // Frame type ZFILE followed by the four information bytes 00 00 00 01, as a real sender
        // wrote them next to these very checksum bytes.
        assert_eq!(crc16(&[0x04, 0x00, 0x00, 0x00, 0x01]), 0x9927);
    }

    #[test]
    fn crc32_of_a_captured_zeof_header_is_0x1c58d0ae_written_low_byte_first() {
        // ZEOF, then the file length 3162 as four little-endian bytes.
        let crc = crc32(&[0x0b, 0x5a, 0x0c, 0x00, 0x00]);
        assert_eq!(crc, 0x1c58_d0ae);
        assert_eq!(crc32_wire_bytes(crc), [0xae, 0xd0, 0x58, 0x1c]);
    }

    #[test]
    fn the_two_checksums_do_not_share_a_wire_byte_order() {
        assert_eq!(crc16_wire_bytes(0x9927), [0x99, 0x27]);
        assert_eq!(crc32_wire_bytes(0x8c92_efbc), [0xbc, 0xef, 0x92, 0x8c]);
    }

    #[test]
    fn chaining_the_slices_is_the_same_as_checksumming_the_whole() {
        let first = b"1234";
        let second = b"56789";
        let mut joined = Vec::new();
        joined.extend_from_slice(first);
        joined.extend_from_slice(second);

        assert_eq!(crc16_update(crc16(first), second), crc16(&joined));
        assert_eq!(
            crc32_update(crc32_update(0xffff_ffff, first), second) ^ 0xffff_ffff,
            crc32(&joined)
        );
    }
}
