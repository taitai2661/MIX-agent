//! Minimal zip reader/writer for Skill bundles.
//!
//! Only the two entry kinds a Skill needs are supported: stored (uncompressed)
//! and deflated. This keeps the dependency surface small while still letting a
//! user round-trip `SKILL.md` plus its `scripts/`, `references/`, and
//! `assets/` files.

use mixd_core::skills::{render_skill, Skill, SkillResource};

/// Read `SKILL.md` and bundled resources out of a zip archive.
pub fn unzip_skill(bytes: &[u8]) -> Result<(String, Vec<SkillResource>), String> {
    let mut body: Option<String> = None;
    let mut resources = Vec::new();
    for entry in read_entries(bytes)? {
        if entry.is_dir {
            continue;
        }
        let path = entry.name.replace('\\', "/");
        if path == "SKILL.md" || path.ends_with("/SKILL.md") {
            body = Some(String::from_utf8_lossy(&entry.data).to_string());
        } else {
            resources.push(SkillResource {
                path,
                content: String::from_utf8_lossy(&entry.data).to_string(),
            });
        }
    }
    let body = body.ok_or("SKILL.md not found in archive")?;
    resources.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((body, resources))
}

/// Read `SKILL.md` and bundled resources from a directory on disk.
pub fn read_skill_dir(dir: &str) -> Result<(String, Vec<SkillResource>), String> {
    let root = std::path::PathBuf::from(dir);
    let skill_path = root.join("SKILL.md");
    let body = std::fs::read_to_string(&skill_path)
        .map_err(|e| format!("cannot read {}: {}", skill_path.display(), e))?;
    let mut resources = Vec::new();
    for sub in ["scripts", "references", "assets"] {
        let subdir = root.join(sub);
        if !subdir.is_dir() {
            continue;
        }
        collect_files(&subdir, &root, &mut resources);
    }
    resources.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((body, resources))
}

fn collect_files(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<SkillResource>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, root, out);
        } else if let Ok(content) = std::fs::read_to_string(&path) {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(SkillResource { path: rel, content });
        }
    }
}

/// Write a Skill back out as a zip archive.
pub fn zip_skill(skill: &Skill) -> Result<Vec<u8>, String> {
    let mut entries: Vec<(String, Vec<u8>)> =
        vec![("SKILL.md".to_string(), render_skill(skill).into_bytes())];
    for resource in &skill.resources {
        entries.push((resource.path.clone(), resource.content.as_bytes().to_vec()));
    }
    write_zip(&entries)
}

struct ZipEntry {
    name: String,
    data: Vec<u8>,
    is_dir: bool,
}

/// Little-endian readers that fail instead of panicking on a truncated archive.
///
/// The previous implementation sliced with `bytes[a..b].try_into().unwrap()`,
/// which is only safe when `b <= len()`. A crafted or truncated zip could make
/// `name_len` or the local-header offsets run past the buffer, and with
/// `panic = "abort"` that took the whole app down.
fn le_u16(bytes: &[u8], at: usize) -> Result<u16, String> {
    let end = at
        .checked_add(2)
        .ok_or_else(|| "zip offset overflow".to_string())?;
    match bytes.get(at..end) {
        Some(b) => Ok(u16::from_le_bytes([b[0], b[1]])),
        None => Err("truncated zip archive".to_string()),
    }
}

fn le_u32(bytes: &[u8], at: usize) -> Result<u32, String> {
    let end = at
        .checked_add(4)
        .ok_or_else(|| "zip offset overflow".to_string())?;
    match bytes.get(at..end) {
        Some(b) => Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        None => Err("truncated zip archive".to_string()),
    }
}

/// Per-entry and per-archive ceilings so a compression bomb cannot exhaust
/// memory before the entry is rejected.
const MAX_ENTRY_BYTES: usize = 64 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;

fn read_entries(bytes: &[u8]) -> Result<Vec<ZipEntry>, String> {
    // Locate the End Of Central Directory record. It occupies the final 22
    // bytes when there is no archive comment, so its signature starts at
    // `len() - 22` inclusive.
    const EOCD_LEN: usize = 22;
    if bytes.len() < EOCD_LEN {
        return Err("not a zip archive".to_string());
    }
    let eocd_sig: u32 = 0x0605_4b50;
    let eocd = (0..=bytes.len() - EOCD_LEN)
        .rev()
        .find(|i| le_u32(bytes, *i).is_ok_and(|sig| sig == eocd_sig))
        .ok_or("not a zip archive")?;
    let total = le_u16(bytes, eocd + 10)? as usize;
    let cd_offset = le_u32(bytes, eocd + 16)? as usize;

    let mut out = Vec::new();
    let mut decoded = 0usize;
    let mut cursor = cd_offset;
    for _ in 0..total {
        if cursor + 46 > bytes.len() {
            break;
        }
        if le_u32(bytes, cursor)? != 0x0201_4b50 {
            break;
        }
        let method = le_u16(bytes, cursor + 10)?;
        let comp_size = le_u32(bytes, cursor + 20)? as usize;
        let name_len = le_u16(bytes, cursor + 28)? as usize;
        let extra_len = le_u16(bytes, cursor + 30)? as usize;
        let comment_len = le_u16(bytes, cursor + 32)? as usize;
        let local_offset = le_u32(bytes, cursor + 42)? as usize;
        let name_start = cursor + 46;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or_else(|| "zip offset overflow".to_string())?;
        if name_end > bytes.len() {
            return Err("truncated zip central directory".to_string());
        }
        let name = String::from_utf8_lossy(&bytes[name_start..name_end]).to_string();

        // Read the local header to find the data offset.
        if local_offset + 30 > bytes.len() {
            break;
        }
        let lh_name_len = le_u16(bytes, local_offset + 26)? as usize;
        let lh_extra_len = le_u16(bytes, local_offset + 28)? as usize;
        let data_start = local_offset
            .checked_add(30)
            .and_then(|v| v.checked_add(lh_name_len))
            .and_then(|v| v.checked_add(lh_extra_len))
            .ok_or_else(|| "zip offset overflow".to_string())?;
        let data_end = data_start
            .checked_add(comp_size)
            .ok_or_else(|| "zip offset overflow".to_string())?;
        if data_start > bytes.len() || data_end > bytes.len() {
            return Err("zip entry data lies outside the archive".to_string());
        }
        let raw = &bytes[data_start..data_end];
        if comp_size > MAX_ENTRY_BYTES {
            return Err("zip entry is too large".to_string());
        }
        let data = if method == 0 {
            raw.to_vec()
        } else if method == 8 {
            inflate(raw)?
        } else {
            return Err(format!("unsupported zip compression method: {}", method));
        };
        if data.len() > MAX_ENTRY_BYTES {
            return Err("zip entry is too large".to_string());
        }
        decoded = decoded.saturating_add(data.len());
        if decoded > MAX_ARCHIVE_BYTES {
            return Err("zip archive expands beyond the size limit".to_string());
        }
        out.push(ZipEntry {
            is_dir: name.ends_with('/'),
            name,
            data,
        });
        cursor += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
}

/// Write a zip archive with stored (uncompressed) entries.
fn write_zip(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let mut count: u16 = 0;

    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0x21u16.to_le_bytes()); // mod date (1980-01-01)
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&0u16.to_le_bytes()); // time
        central.extend_from_slice(&0x21u16.to_le_bytes()); // date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
        count += 1;
    }

    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    Ok(out)
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *slot = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc = table[((crc ^ *byte as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Raw DEFLATE decompression for zip entries that use method 8.
fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut reader = BitReader {
        data,
        pos: 0,
        bit: 0,
    };
    loop {
        let final_block = reader.bits(1)? == 1;
        let btype = reader.bits(2)?;
        match btype {
            0 => {
                reader.align();
                let len = reader.bits(16)? as usize;
                let _nlen = reader.bits(16)?;
                if out.len() + len > MAX_ENTRY_BYTES {
                    return Err("deflate output exceeds the size limit".into());
                }
                for _ in 0..len {
                    out.push(reader.bits(8)? as u8);
                }
            }
            1 => {
                let (lit, dist) = fixed_trees();
                inflate_block(&mut reader, &lit, &dist, &mut out, MAX_ENTRY_BYTES)?;
            }
            2 => {
                let (lit, dist) = dynamic_trees(&mut reader)?;
                inflate_block(&mut reader, &lit, &dist, &mut out, MAX_ENTRY_BYTES)?;
            }
            _ => return Err("invalid deflate block type".into()),
        }
        if final_block {
            break;
        }
    }
    Ok(out)
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u8,
}

impl<'a> BitReader<'a> {
    fn bits(&mut self, count: u8) -> Result<u32, String> {
        let mut value = 0u32;
        for i in 0..count {
            if self.pos >= self.data.len() {
                return Err("unexpected end of deflate stream".into());
            }
            let b = (self.data[self.pos] >> self.bit) & 1;
            value |= (b as u32) << i;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        Ok(value)
    }

    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }
}

struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Self {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offsets = [0u16; 16];
        for i in 1..16 {
            offsets[i] = offsets[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offsets[l as usize] as usize] = sym as u16;
                offsets[l as usize] += 1;
            }
        }
        Self { counts, symbols }
    }

    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, String> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for len in 1..16 {
            code |= reader.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("invalid huffman code".into())
    }
}

fn fixed_trees() -> (Huffman, Huffman) {
    let mut lit_lengths = vec![8u8; 288];
    for item in lit_lengths.iter_mut().take(256).skip(144) {
        *item = 9;
    }
    for item in lit_lengths.iter_mut().take(280).skip(256) {
        *item = 7;
    }
    let dist_lengths = vec![5u8; 30];
    (Huffman::new(&lit_lengths), Huffman::new(&dist_lengths))
}

fn dynamic_trees(reader: &mut BitReader<'_>) -> Result<(Huffman, Huffman), String> {
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let hlit = reader.bits(5)? as usize + 257;
    let hdist = reader.bits(5)? as usize + 1;
    let hclen = reader.bits(4)? as usize + 4;
    let mut code_lengths = [0u8; 19];
    for &idx in ORDER.iter().take(hclen) {
        code_lengths[idx] = reader.bits(3)? as u8;
    }
    let code_tree = Huffman::new(&code_lengths);
    let mut lengths = vec![0u8; hlit + hdist];
    let mut i = 0;
    while i < lengths.len() {
        let sym = code_tree.decode(reader)?;
        match sym {
            0..=15 => {
                lengths[i] = sym as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err("repeat with no previous length".into());
                }
                let prev = lengths[i - 1];
                let repeat = 3 + reader.bits(2)? as usize;
                for _ in 0..repeat {
                    if i >= lengths.len() {
                        break;
                    }
                    lengths[i] = prev;
                    i += 1;
                }
            }
            17 => {
                let repeat = 3 + reader.bits(3)? as usize;
                i = (i + repeat).min(lengths.len());
            }
            18 => {
                let repeat = 11 + reader.bits(7)? as usize;
                i = (i + repeat).min(lengths.len());
            }
            _ => return Err("invalid code-length symbol".into()),
        }
    }
    Ok((
        Huffman::new(&lengths[..hlit]),
        Huffman::new(&lengths[hlit..]),
    ))
}

fn inflate_block(
    reader: &mut BitReader<'_>,
    lit: &Huffman,
    dist: &Huffman,
    out: &mut Vec<u8>,
    limit: usize,
) -> Result<(), String> {
    const LENGTH_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const LENGTH_EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    const DIST_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const DIST_EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];

    loop {
        // Checked once per symbol: a crafted stream can expand by ~1000x, so
        // waiting until the block ends would already have allocated it.
        if out.len() > limit {
            return Err("deflate output exceeds the size limit".into());
        }
        let sym = lit.decode(reader)?;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let idx = (sym - 257) as usize;
            if idx >= LENGTH_BASE.len() {
                return Err("invalid length symbol".into());
            }
            let length = LENGTH_BASE[idx] as usize + reader.bits(LENGTH_EXTRA[idx])? as usize;
            let dsym = dist.decode(reader)? as usize;
            if dsym >= DIST_BASE.len() {
                return Err("invalid distance symbol".into());
            }
            let distance = DIST_BASE[dsym] as usize + reader.bits(DIST_EXTRA[dsym])? as usize;
            if distance > out.len() {
                return Err("distance exceeds output".into());
            }
            let start = out.len() - distance;
            for i in 0..length {
                let byte = out[start + i];
                out.push(byte);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_stored_entries() {
        let entries = vec![
            ("SKILL.md".to_string(), b"name: demo\n".to_vec()),
            ("scripts/run.py".to_string(), b"print(1)\n".to_vec()),
        ];
        let bytes = write_zip(&entries).expect("write");
        let (body, resources) = unzip_skill(&bytes).expect("read");
        assert_eq!(body, "name: demo\n");
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].path, "scripts/run.py");
    }

    #[test]
    fn crc32_matches_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn round_trips_stored_entry_against_system_unzip() {
        // Guard the archive layout: a real `unzip` must accept what we write.
        let entries = vec![
            ("SKILL.md".to_string(), b"name: demo\n".to_vec()),
            ("scripts/run.py".to_string(), b"print(1)\n".to_vec()),
        ];
        let bytes = write_zip(&entries).expect("write");
        let path = std::env::temp_dir().join(format!("mix-skill-{}.zip", std::process::id()));
        std::fs::write(&path, &bytes).expect("write temp");
        let out = std::process::Command::new("unzip")
            .arg("-l")
            .arg(&path)
            .output()
            .expect("run unzip");
        let _ = std::fs::remove_file(&path);
        assert!(
            out.status.success(),
            "system unzip rejected our archive: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn inflates_stored_deflate_block() {
        // A raw DEFLATE stream: final stored block of 5 bytes "hello".
        let mut data = vec![0x01, 0x05, 0x00, 0xfa, 0xff];
        data.extend_from_slice(b"hello");
        assert_eq!(inflate(&data).expect("inflate"), b"hello");
    }

    #[test]
    fn inflates_fixed_huffman_block() {
        // zlib-compressed "aaaaaaaaaa" (fixed Huffman with back-references).
        let raw = miniz_oxide_deflate(b"aaaaaaaaaa");
        assert_eq!(inflate(&raw).expect("inflate"), b"aaaaaaaaaa");
    }

    /// Produce a raw DEFLATE stream using the system `python3` zlib module so
    /// the hand-rolled inflate implementation is tested against real data.
    fn miniz_oxide_deflate(input: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut script = String::new();
        script.push_str("import sys,zlib\n");
        script.push_str("co=zlib.compressobj(9,zlib.DEFLATED,-15)\n");
        script
            .push_str("sys.stdout.buffer.write(co.compress(sys.stdin.buffer.read())+co.flush())\n");
        let mut child = std::process::Command::new("python3")
            .arg("-c")
            .arg(&script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn python3");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(input)
            .expect("write stdin");
        let out = child.wait_with_output().expect("wait");
        assert!(out.status.success(), "python3 zlib failed");
        out.stdout
    }

    #[test]
    fn a_truncated_archive_is_rejected_without_panicking() {
        let entries = vec![("SKILL.md".to_string(), b"name: demo\n".to_vec())];
        let bytes = write_zip(&entries).expect("write");
        // Every prefix must produce a Result: slicing past the buffer used to
        // panic, which with `panic = "abort"` killed the process.
        for cut in 0..bytes.len() {
            let _ = unzip_skill(&bytes[..cut]);
        }
    }

    #[test]
    fn a_name_length_that_runs_past_the_buffer_is_rejected() {
        let entries = vec![("SKILL.md".to_string(), b"name: demo\n".to_vec())];
        let mut bytes = write_zip(&entries).expect("write");
        // Central directory header: name length lives at offset 28.
        let eocd = bytes.len() - EOCD_FOR_TEST;
        let cd = u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
        bytes[cd + 28] = 0xff;
        bytes[cd + 29] = 0xff;
        assert!(unzip_skill(&bytes).is_err());
    }

    #[test]
    fn entry_data_outside_the_archive_is_rejected() {
        let entries = vec![("SKILL.md".to_string(), b"name: demo\n".to_vec())];
        let mut bytes = write_zip(&entries).expect("write");
        let eocd = bytes.len() - EOCD_FOR_TEST;
        let cd = u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
        // Claim a compressed size far beyond the archive.
        bytes[cd + 20] = 0xff;
        bytes[cd + 21] = 0xff;
        bytes[cd + 22] = 0xff;
        bytes[cd + 23] = 0x00;
        assert!(unzip_skill(&bytes).is_err());
    }

    /// End-of-central-directory record length, used when poking at headers.
    const EOCD_FOR_TEST: usize = 22;
}
