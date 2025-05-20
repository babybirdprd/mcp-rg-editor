use tracing::debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEndingStyle {
    Lf,    // \n
    CrLf,  // \r\n
    Cr,    // \r
    Mixed, // Indicates a mix, though normalization usually picks one
    Unknown, // Could not determine or empty content
}

impl LineEndingStyle {
    pub fnas_str(&self) -> &'static str {
        match self {
            LineEndingStyle::Lf => "\n",
            LineEndingStyle::CrLf => "\r\n",
            LineEndingStyle::Cr => "\r",
            LineEndingStyle::Mixed | LineEndingStyle::Unknown => {
                // Default to system's line ending if mixed or unknown for writing
                if cfg!(windows) { "\r\n" } else { "\n" }
            }
        }
    }
}

/// Detects the predominant line ending style in a string.
pub fn detect_line_ending(content: &str) -> LineEndingStyle {
    if content.is_empty() {
        return LineEndingStyle::Unknown;
    }

    let mut lf_count = 0;
    let mut crlf_count = 0;
    let mut cr_count = 0;

    let mut i = 0;
    let bytes = content.as_bytes();
    let len = bytes.len();

    while i < len {
        if bytes[i] == b'\r' {
            if i + 1 < len && bytes[i + 1] == b'\n' {
                crlf_count += 1;
                i += 2;
            } else {
                cr_count += 1;
                i += 1;
            }
        } else if bytes[i] == b'\n' {
            lf_count += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    
    debug!(lf=lf_count, crlf=crlf_count, cr=cr_count, "Detected line ending counts");

    if crlf_count > 0 && lf_count == 0 && cr_count == 0 {
        return LineEndingStyle::CrLf;
    }
    if lf_count > 0 && crlf_count == 0 && cr_count == 0 {
        return LineEndingStyle::Lf;
    }
    if cr_count > 0 && crlf_count == 0 && lf_count == 0 {
        return LineEndingStyle::Cr;
    }

    // Determine predominant if mixed
    if crlf_count >= lf_count && crlf_count >= cr_count && crlf_count > 0 {
        return LineEndingStyle::CrLf; // Prioritize CRLF if it's the max
    }
    if lf_count >= crlf_count && lf_count >= cr_count && lf_count > 0 {
        return LineEndingStyle::Lf; // Then LF
    }
    if cr_count > 0 { // Then CR
        return LineEndingStyle::Cr;
    }
    
    // If counts are present but not decisive (e.g. mixed with no clear dominant)
    if crlf_count > 0 || lf_count > 0 || cr_count > 0 {
        // Default to LF in truly mixed scenarios without a clear winner, or system default.
        // For simplicity, let's pick LF as a common intermediate.
        debug!("Mixed line endings detected, defaulting to LF for normalization purposes if needed.");
        return LineEndingStyle::Mixed; // Indicate mixed, let caller decide or use system default.
    }

    LineEndingStyle::Unknown // No line endings found
}

/// Normalizes all line endings in a string to the specified style.
pub fn normalize_line_endings(text: &str, target_style: LineEndingStyle) -> String {
    if target_style == LineEndingStyle::Unknown || target_style == LineEndingStyle::Mixed {
        // If target is unknown/mixed, don't change, or normalize to system default
        let system_default = if cfg!(windows) { LineEndingStyle::CrLf } else { LineEndingStyle::Lf };
        return normalize_line_endings(text, system_default);
    }

    // First, normalize all known line endings to LF (\n)
    let normalized_to_lf = text.replace("\r\n", "\n").replace('\r', "\n");

    // Then, convert from LF to the target style
    match target_style {
        LineEndingStyle::Lf => normalized_to_lf,
        LineEndingStyle::CrLf => normalized_to_lf.replace('\n', "\r\n"),
        LineEndingStyle::Cr => normalized_to_lf.replace('\n', "\r"),
        _ => normalized_to_lf, // Should not happen due to guard above
    }
}