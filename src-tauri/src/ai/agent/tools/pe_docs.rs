//! PE (Android/iOS) document-only file policy.
//!
//! Desktop tools keep the full workspace. On phone, Write/Edit/Read/Delete
//! and the reader file tree only deal with Markdown / plain-text drafts.

use std::path::Path;

use crate::error::{AppError, AppResult};

pub const DOC_EXTENSIONS: &[&str] = &["md", "txt", "markdown"];

pub fn pe_docs_only() -> bool {
    crate::data::paths::pe_platform()
}

pub fn is_doc_extension(ext: &str) -> bool {
    DOC_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

pub fn is_doc_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(is_doc_extension)
        .unwrap_or(false)
}

/// Refuse a non-document file on PE. Directories are always allowed.
pub fn refuse_nondoc(tool: &str, path: &Path) -> AppResult<()> {
    if !pe_docs_only() || path.is_dir() || is_doc_path(path) {
        return Ok(());
    }
    Err(AppError::Invalid(format!(
        "{tool}: on this device only Markdown/plain-text documents (`.md`, `.txt`) are allowed: {}",
        path.display()
    )))
}

/// When listing, keep directories; skip non-document files on PE.
pub fn include_listed_path(path: &Path, is_dir: bool) -> bool {
    is_dir || !pe_docs_only() || is_doc_path(path)
}
