// FILE: src-tauri/src/utils/line_ending_handler.rs
// IMPORTANT NOTE: Rewrite the entire file.
use tracing::debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum LineEndingStyle {
    Lf,
    CrLf,
    Cr,
    Mixed,
    Unknown,
}

impl LineEndingStyle {
    pub fn as_str(&self) -> &'static str {
        match self {
            LineEndingStyle::Lf => "\n",
            LineEndingStyle::CrLf => "\r\n",
            LineEndingStyle::Cr => "\r",
            LineEndingStyle::Mixed | LineEndingStyle::Unknown => {
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
    
    if crlf_count == 0 && lf_count == 0 && cr_count == 0 {
        return LineEndingStyle::Unknown;
    }

    if crlf_count >= lf_count && crlf_count >= cr_count {
        return LineEndingStyle::CrLf;
    }
    if lf_count >= crlf_count && lf_count >= cr_count {
        return LineEndingStyle::Lf;
    }
    return LineEndingStyle::Cr;
}

pub fn normalize_line_endings(text: &str, target_style: LineEndingStyle) -> String {
    let effective_target_style = match target_style {
        LineEndingStyle::Unknown | LineEndingStyle::Mixed => {
            if cfg!(windows) { LineEndingStyle::CrLf } else { LineEndingStyle::Lf }
        },
        _ => target_style,
    };

    let normalized_to_lf = text.replace("\r\n", "\n").replace('\r', "\n");

    match effective_target_style {
        LineEndingStyle::Lf => normalized_to_lf,
        LineEndingStyle::CrLf => normalized_to_lf.replace('\n', "\r\n"),
        LineEndingStyle::Cr => normalized_to_lf.replace('\n', "\r"),
        _ => normalized_to_lf,
    }
}