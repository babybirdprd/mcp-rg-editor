use tracing::debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] // Added Serialize
pub enum LineEndingStyle {
    Lf,
    CrLf,
    Cr,
    Mixed, // Indicates multiple types found, but a primary one might be chosen
    Unknown, // No line endings found or indeterminate
}

impl LineEndingStyle {
    pub fn as_str(&self) -> &'static str {
        match self {
            LineEndingStyle::Lf => "\n",
            LineEndingStyle::CrLf => "\r\n",
            LineEndingStyle::Cr => "\r",
            LineEndingStyle::Mixed | LineEndingStyle::Unknown => {
                // Default to system's preference if mixed or unknown
                if cfg!(windows) { "\r\n" } else { "\n" }
            }
        }
    }
}

pub fn detect_line_ending(content: &str) -> LineEndingStyle {
    if content.is_empty() {
        return LineEndingStyle::Unknown;
    }

    let mut lf_count = 0;
    let mut crlf_count = 0;
    let mut cr_count = 0; // Standalone CRs

    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                crlf_count += 1;
                i += 2; // Skip both \r and \n
            } else {
                cr_count += 1;
                i += 1; // Skip \r
            }
        } else if bytes[i] == b'\n' {
            lf_count += 1;
            i += 1; // Skip \n
        } else {
            i += 1;
        }
    }

    debug!(lf = lf_count, crlf = crlf_count, cr = cr_count, "Detected line ending counts");

    if crlf_count > 0 && lf_count == 0 && cr_count == 0 {
        LineEndingStyle::CrLf
    } else if lf_count > 0 && crlf_count == 0 && cr_count == 0 {
        LineEndingStyle::Lf
    } else if cr_count > 0 && crlf_count == 0 && lf_count == 0 {
        LineEndingStyle::Cr
    } else if crlf_count == 0 && lf_count == 0 && cr_count == 0 {
        LineEndingStyle::Unknown // No line endings found
    } else {
        // Mixed line endings found. Determine predominant or default.
        // This simplistic approach picks the most frequent.
        // A more sophisticated approach might be needed for true "Mixed" handling.
        if crlf_count >= lf_count && crlf_count >= cr_count {
            debug!("Mixed line endings detected, defaulting to CRLF due to predominance.");
            LineEndingStyle::CrLf // Or LineEndingStyle::Mixed if you want to flag it
        } else if lf_count >= crlf_count && lf_count >= cr_count {
            debug!("Mixed line endings detected, defaulting to LF due to predominance.");
            LineEndingStyle::Lf // Or LineEndingStyle::Mixed
        } else {
            debug!("Mixed line endings detected, defaulting to CR due to predominance.");
            LineEndingStyle::Cr // Or LineEndingStyle::Mixed
        }
    }
}

pub fn normalize_line_endings(text: &str, target_style: LineEndingStyle) -> String {
    // First, normalize all known line endings to LF
    let normalized_to_lf = text.replace("\r\n", "\n").replace('\r', "\n");

    // Then, convert LFs to the target style
    match target_style {
        LineEndingStyle::Lf => normalized_to_lf,
        LineEndingStyle::CrLf => normalized_to_lf.replace('\n', "\r\n"),
        LineEndingStyle::Cr => normalized_to_lf.replace('\n', "\r"),
        LineEndingStyle::Mixed | LineEndingStyle::Unknown => {
            // If target is mixed or unknown, use system default
            if cfg!(windows) {
                normalized_to_lf.replace('\n', "\r\n")
            } else {
                normalized_to_lf
            }
        }
    }
}