//! `{}`-placeholder string formatting.
//!
//! Ported from `util/string/format.{h,cpp}`. The C++ `format()` substitutes `{}`
//! placeholders left-to-right into a `TextWriter`. Rust has `format!`, but this
//! module exposes a runtime (non-macro) equivalent for API parity with C++ call
//! sites that build format strings dynamically, plus the dev-only `to_hex_table`.

/// Substitute `{}` placeholders in `fmt` left-to-right with the string forms of
/// `args`. If there are more args than placeholders, the remainder is appended as
/// ` [...]`; if fewer, trailing `{}` are left verbatim (matching the C++ writer
/// behaviour where leftover format text is written as-is at the end).
///
/// Mirrors `zylann::format(fmt, args...)`.
pub fn format<I, S>(fmt: &str, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = String::with_capacity(fmt.len() + 8);
    let mut rest = fmt;
    let mut consumed_any = false;
    for a in args {
        consumed_any = true;
        match rest.find("{}") {
            Some(pi) => {
                out.push_str(&rest[..pi]);
                out.push_str(a.as_ref());
                rest = &rest[pi + 2..];
            }
            None => {
                // Too many arguments: write leftover text + sentinel, stop.
                out.push_str(rest);
                out.push_str(" [...]");
                return out;
            }
        }
    }
    // Write whatever format text remains (covers the "fewer args" case too).
    out.push_str(rest);
    let _ = consumed_any;
    out
}

/// Render `data` as a classic hex-dump table (offset | hex bytes, 16 per row).
/// Dev/debug helper. Matches `to_hex_table` (DEV_ENABLED in C++).
pub fn to_hex_table(data: &[u8]) -> String {
    fn to_hex(nibble: u8) -> char {
        if nibble < 10 {
            (b'0' + nibble) as char
        } else {
            (b'a' + (nibble - 10)) as char
        }
    }
    let mut ss = String::new();
    ss.push_str("---\n");
    ss.push_str("Data size: ");
    ss.push_str(&data.len().to_string());
    for (i, &b) in data.iter().enumerate() {
        if i % 16 == 0 {
            ss.push('\n');
            ss.push_str(&i.to_string());
            // Right-pad the offset to a fixed width (margin 6) with spaces.
            let margin = 6;
            let mut p = 10usize;
            for _ in 0..margin {
                if i < p {
                    ss.push(' ');
                }
                p *= 10;
            }
            ss.push_str(" | ");
        }
        let low = b & 0xf;
        let high = (b >> 4) & 0xf;
        ss.push(to_hex(high));
        ss.push(to_hex(low));
        ss.push(' ');
    }
    ss.push('\n');
    ss.push_str("---");
    ss
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_substitutes_in_order() {
        assert_eq!(format("hello {}", ["world"]), "hello world");
        assert_eq!(format("{} + {} = {}", ["1", "2", "3"]), "1 + 2 = 3");
    }

    #[test]
    fn format_extra_args_appended() {
        // More args than placeholders -> leftover text + " [...]"
        assert_eq!(format("hi {}", ["a", "b"]), "hi a [...]");
    }

    #[test]
    fn format_fewer_args_leaves_placeholder() {
        // Fewer args than placeholders -> remaining {} written verbatim.
        // First {} consumed with "one", " and " written, trailing "{}" stays.
        assert_eq!(format("{} and {}", ["one"]), "one and {}");
    }

    #[test]
    fn format_no_placeholders() {
        // No placeholders at all with no args -> verbatim.
        assert_eq!(format("plain", Vec::<&str>::new()), "plain");
    }

    #[test]
    fn hex_table_layout() {
        let data = [0x00, 0x1f, 0xff, 0xab];
        let table = to_hex_table(&data);
        // Header + size line.
        assert!(table.starts_with("---\nData size: 4\n"));
        // Each byte rendered as two hex digits + space.
        assert!(table.contains("00 1f ff ab "));
        assert!(table.ends_with("\n---"));
    }

    #[test]
    fn hex_table_row_boundaries() {
        // 17 bytes -> two rows (16 + 1), second row offset is 16.
        let data: Vec<u8> = (0..17).collect();
        let table = to_hex_table(&data);
        assert!(table.contains("\n16      | 10 "));
    }
}
