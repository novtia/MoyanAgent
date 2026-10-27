//! Concatenate selected project `.txt` files into one UTF-8 text file or a
//! minimal Word document. Each file is introduced by its name, then its body.
//! Order is the natural filename order, not the order the files were clicked.

use std::fs;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use tauri::State;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::ai::agent::tools::natural_name::cmp_natural_path;
use crate::ai::agent::tools::text_decode::read_text_file;
use crate::error::{AppError, AppResult};

use super::reader_paths::{session_project_cwd, validate_reader_write_path};
use super::state::AppState;

/// Stack each file as its name, then its body:
///
/// ```text
/// 第一章
/// 正文
/// 第二章
/// 正文
/// ```
pub(crate) fn join_documents(parts: &[(String, String)]) -> String {
    let mut out = String::new();
    for (name, body) in parts {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(name);
        out.push('\n');
        let body = body.trim_end_matches(['\r', '\n']);
        if !body.is_empty() {
            out.push_str(body);
            out.push('\n');
        }
    }
    out
}

#[tauri::command]
pub fn export_project_texts(
    state: State<'_, std::sync::Arc<AppState>>,
    session_id: String,
    paths: Vec<String>,
    dest_path: String,
    format: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let cwd = session_project_cwd(&conn, &session_id);
    let mut files = Vec::new();
    for raw in &paths {
        let resolved = validate_reader_write_path(&PathBuf::from(raw), cwd.as_deref())?;
        if !resolved.is_file() {
            continue;
        }
        let is_txt = resolved
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"));
        if !is_txt {
            continue;
        }
        files.push(resolved);
    }
    if files.is_empty() {
        return Err(AppError::Invalid(
            "export_project_texts: no txt files selected".into(),
        ));
    }
    files.sort_by(|a, b| cmp_natural_path(a, b));

    let mut parts = Vec::with_capacity(files.len());
    for path in &files {
        let decoded = read_text_file(path)
            .map_err(|e| AppError::Other(format!("export_project_texts: read {:?}: {e}", path)))?;
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        parts.push((name, decoded.text));
    }
    let text = join_documents(&parts);
    let dest = PathBuf::from(&dest_path);
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                AppError::Other(format!("export_project_texts: mkdir {:?}: {e}", parent))
            })?;
        }
    }
    let bytes = match format.trim().to_ascii_lowercase().as_str() {
        "docx" | "word" => docx_bytes(&text)?,
        _ => text.into_bytes(),
    };
    fs::write(&dest, bytes)
        .map_err(|e| AppError::Other(format!("export_project_texts: write {:?}: {e}", dest)))?;
    Ok(())
}

fn docx_bytes(text: &str) -> AppResult<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(&mut cursor);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    write_zip(
        &mut zip,
        opts,
        "[Content_Types].xml",
        CONTENT_TYPES.as_bytes(),
    )?;
    write_zip(&mut zip, opts, "_rels/.rels", PACKAGE_RELS.as_bytes())?;
    write_zip(
        &mut zip,
        opts,
        "word/_rels/document.xml.rels",
        DOCUMENT_RELS.as_bytes(),
    )?;
    let document = document_xml(text);
    write_zip(&mut zip, opts, "word/document.xml", document.as_bytes())?;
    zip.finish()
        .map_err(|e| AppError::Other(format!("export_project_texts: docx finish: {e}")))?;
    Ok(cursor.into_inner())
}

fn write_zip<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    opts: SimpleFileOptions,
    name: &str,
    bytes: &[u8],
) -> AppResult<()> {
    zip.start_file(name, opts)
        .map_err(|e| AppError::Other(format!("export_project_texts: docx {name}: {e}")))?;
    zip.write_all(bytes)
        .map_err(|e| AppError::Other(format!("export_project_texts: docx {name}: {e}")))?;
    Ok(())
}

fn document_xml(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.strip_suffix('\n').unwrap_or(&normalized);
    let mut body = String::new();
    for line in normalized.split('\n') {
        body.push_str("<w:p><w:r><w:t xml:space=\"preserve\">");
        body.push_str(&xml_escape(line));
        body.push_str("</w:t></w:r></w:p>");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
         <w:body>{body}<w:sectPr/></w:body></w:document>"
    )
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c.is_control() && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
</Types>";

const PACKAGE_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";

const DOCUMENT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>";

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn documents_are_separated_by_filename() {
        let joined = join_documents(&[
            ("第一章".to_string(), "第一章正文\n".to_string()),
            ("第二章".to_string(), "第二章正文".to_string()),
            ("第三章".to_string(), "第三章正文\r\n".to_string()),
        ]);
        assert_eq!(
            joined,
            "第一章\n第一章正文\n第二章\n第二章正文\n第三章\n第三章正文\n"
        );
    }

    #[test]
    fn docx_contains_the_joined_paragraphs_in_order() {
        let text = join_documents(&[
            ("第一章".to_string(), "甲的正文".to_string()),
            ("第二章".to_string(), "乙的正文".to_string()),
        ]);
        let bytes = docx_bytes(&text).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        let first_title = xml.find("第一章").unwrap();
        let first = xml.find("甲的正文").unwrap();
        let second_title = xml.find("第二章").unwrap();
        let second = xml.find("乙的正文").unwrap();
        assert!(first_title < first && first < second_title && second_title < second, "{xml}");
        assert!(xml.contains("<w:p>"));
    }
}
