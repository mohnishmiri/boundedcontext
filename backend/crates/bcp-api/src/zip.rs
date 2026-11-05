//! Minimal ZIP (PKWARE APPNOTE 6.3) writer for in-memory archives of generated text files: deflate
//! compression, UTF-8 names, no ZIP64 (the API caps archives far below the 4 GiB format limit).

use std::io::Write;

use flate2::write::DeflateEncoder;
use flate2::{Compression, Crc};

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_CENTRAL_DIR: u32 = 0x0605_4b50;
const VERSION: u16 = 20;
/// Bit 11: file names are UTF-8.
const FLAG_UTF8: u16 = 1 << 11;
const METHOD_DEFLATE: u16 = 8;
/// MS-DOS date/time 1980-01-01 00:00 (archives are reproducible; the job page shows the real dates).
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

struct Entry {
    name: Vec<u8>,
    crc: u32,
    compressed: u32,
    size: u32,
    offset: u32,
}

#[derive(Default)]
pub struct ZipWriter {
    out: Vec<u8>,
    entries: Vec<Entry>,
}

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn u32le(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn to_u32(n: usize) -> std::io::Result<u32> {
    u32::try_from(n).map_err(|_| std::io::Error::other("archive too large"))
}

impl ZipWriter {
    pub fn add(&mut self, name: &str, data: &[u8]) -> std::io::Result<()> {
        let mut crc = Crc::new();
        crc.update(data);
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data)?;
        let compressed = enc.finish()?;
        let entry = Entry {
            name: name.as_bytes().to_vec(),
            crc: crc.sum(),
            compressed: to_u32(compressed.len())?,
            size: to_u32(data.len())?,
            offset: to_u32(self.out.len())?,
        };
        let o = &mut self.out;
        u32le(o, LOCAL_HEADER);
        u16le(o, VERSION);
        u16le(o, FLAG_UTF8);
        u16le(o, METHOD_DEFLATE);
        u16le(o, DOS_TIME);
        u16le(o, DOS_DATE);
        u32le(o, entry.crc);
        u32le(o, entry.compressed);
        u32le(o, entry.size);
        u16le(o, entry.name.len() as u16);
        u16le(o, 0);
        o.extend_from_slice(&entry.name);
        o.extend_from_slice(&compressed);
        self.entries.push(entry);
        Ok(())
    }

    pub fn finish(mut self) -> std::io::Result<Vec<u8>> {
        let start = to_u32(self.out.len())?;
        let o = &mut self.out;
        for e in &self.entries {
            u32le(o, CENTRAL_HEADER);
            u16le(o, VERSION);
            u16le(o, VERSION);
            u16le(o, FLAG_UTF8);
            u16le(o, METHOD_DEFLATE);
            u16le(o, DOS_TIME);
            u16le(o, DOS_DATE);
            u32le(o, e.crc);
            u32le(o, e.compressed);
            u32le(o, e.size);
            u16le(o, e.name.len() as u16);
            u16le(o, 0); // extra field length
            u16le(o, 0); // comment length
            u16le(o, 0); // disk number start
            u16le(o, 0); // internal attributes
            u32le(o, 0); // external attributes
            u32le(o, e.offset);
            o.extend_from_slice(&e.name);
        }
        let size = to_u32(o.len())? - start;
        let count = u16::try_from(self.entries.len()).map_err(|_| std::io::Error::other("too many files"))?;
        u32le(o, END_OF_CENTRAL_DIR);
        u16le(o, 0);
        u16le(o, 0);
        u16le(o, count);
        u16le(o, count);
        u32le(o, size);
        u32le(o, start);
        u16le(o, 0);
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::DeflateDecoder;

    use super::*;

    fn le16(b: &[u8], at: usize) -> usize {
        u16::from_le_bytes([b[at], b[at + 1]]) as usize
    }

    fn le32(b: &[u8], at: usize) -> usize {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
    }

    #[test]
    fn writes_a_readable_archive() {
        let mut z = ZipWriter::default();
        z.add("shop/docs/BOUNDED-CONTEXT-shop.md", "# Shop\n\nOrders and billing.\n".repeat(50).as_bytes()).unwrap();
        z.add("shop/docs/archetypes/00-INDEX.md", b"index").unwrap();
        let bytes = z.finish().unwrap();

        // End of central directory: two entries, central directory where it says it is.
        let eocd = bytes.len() - 22;
        assert_eq!(le32(&bytes, eocd), END_OF_CENTRAL_DIR as usize);
        assert_eq!(le16(&bytes, eocd + 10), 2);
        let cd = le32(&bytes, eocd + 16);
        assert_eq!(le32(&bytes, cd), CENTRAL_HEADER as usize);

        // First local entry decompresses to the original with a matching CRC.
        assert_eq!(le32(&bytes, 0), LOCAL_HEADER as usize);
        let name_len = le16(&bytes, 26);
        let comp_len = le32(&bytes, 18);
        assert_eq!(&bytes[30..30 + name_len], b"shop/docs/BOUNDED-CONTEXT-shop.md");
        let data = &bytes[30 + name_len..30 + name_len + comp_len];
        let mut out = String::new();
        DeflateDecoder::new(data).read_to_string(&mut out).unwrap();
        assert_eq!(out, "# Shop\n\nOrders and billing.\n".repeat(50));
        let mut crc = Crc::new();
        crc.update(out.as_bytes());
        assert_eq!(le32(&bytes, 14), crc.sum() as usize);
    }
}
