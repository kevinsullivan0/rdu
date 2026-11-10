use std::ffi::OsStr;
pub fn safe_text(name: &OsStr) -> String {
    escaped(name)
}

fn escaped(name: &OsStr) -> String {
    use std::fmt::Write;
    use std::os::unix::ffi::OsStrExt;
    let mut output = String::new();
    let mut bytes = name.as_bytes();
    while !bytes.is_empty() {
        let (valid, invalid) = match std::str::from_utf8(bytes) {
            Ok(s) => (s, 0),
            Err(e) => (
                std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap(),
                e.error_len().unwrap_or(bytes.len() - e.valid_up_to()),
            ),
        };
        for ch in valid.chars() {
            if ch == '\\' {
                output.push_str("\\\\");
            } else if ch.is_control()
                || matches!(ch as u32, 0xad | 0x61c | 0x200b..=0x200f | 0x2028..=0x202e | 0x2060..=0x206f | 0xfeff | 0xfff9..=0xfffb)
            {
                if (ch as u32) < 256 {
                    write!(output, "\\x{:02x}", ch as u32).unwrap();
                } else {
                    write!(output, "\\u{{{:x}}}", ch as u32).unwrap();
                }
            } else {
                output.push(ch);
            }
        }
        bytes = &bytes[valid.len()..];
        for byte in &bytes[..invalid] {
            write!(output, "\\x{byte:02x}").unwrap();
        }
        bytes = &bytes[invalid..];
    }
    output
}

pub(super) fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
