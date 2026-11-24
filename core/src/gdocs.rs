//! Google-format files on disk: link files (Insync's JSON format and
//! extensions, so an Insync folder can be adopted) or converted exports.

use crate::cloud::CloudItem;
use crate::config::ExportOption;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const GA: &str = "application/vnd.google-apps.";

/// (Google type, link extension, Office (ext, mime), OpenDocument (ext, mime)).
type Fmt = (&'static str, &'static str, Option<(&'static str, &'static str)>, Option<(&'static str, &'static str)>);

const FORMATS: &[Fmt] = &[
    ("document", "gddoc", Some(("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document")), Some(("odt", "application/vnd.oasis.opendocument.text"))),
    ("spreadsheet", "gdsheet", Some(("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")), Some(("ods", "application/vnd.oasis.opendocument.spreadsheet"))),
    ("presentation", "gdslides", Some(("pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation")), Some(("odp", "application/vnd.oasis.opendocument.presentation"))),
    ("drawing", "gddraw", Some(("svg", "image/svg+xml")), Some(("svg", "image/svg+xml"))),
    ("form", "gdform", None, None),
    ("script", "gdscript", None, None),
    ("map", "gdmap", None, None),
    ("fusiontable", "gdtable", None, None),
    ("jam", "gdjam", None, None),
    ("site", "gdsite", None, None),
    ("shortcut", "gdlink", None, None),
];

/// Every link-file extension (canopy's and Insync's are the same).
pub fn link_extensions() -> Vec<&'static str> {
    FORMATS.iter().map(|f| f.1).collect()
}

pub fn is_link_name(name: &str) -> bool {
    name.rsplit_once('.').map(|(_, e)| FORMATS.iter().any(|f| f.1 == e)).unwrap_or(false)
}

fn fmt_for(mime: &str) -> Option<&'static Fmt> {
    let kind = mime.strip_prefix(GA)?;
    FORMATS.iter().find(|f| f.0 == kind)
}

/// How a Google-format file is kept on disk.
#[derive(Clone, Debug, PartialEq)]
pub enum Local {
    /// A link file with this extension.
    Link(&'static str),
    /// An export: (extension, MIME type to export as and upload back from).
    Export(&'static str, &'static str),
}

pub fn local_form(item: &CloudItem, opt: ExportOption) -> Local {
    let f = fmt_for(&item.mime);
    let conv = match (opt, f) {
        (ExportOption::MsOffice, Some(f)) => f.2,
        (ExportOption::OpenDocument, Some(f)) => f.3,
        _ => None,
    };
    match conv {
        Some((ext, mime)) => Local::Export(ext, mime),
        None => Local::Link(f.map(|f| f.1).unwrap_or("gdlink")),
    }
}

/// The local file name of a Google-format file ("Budget" -> "Budget.gdsheet").
pub fn local_name(item: &CloudItem, opt: ExportOption) -> String {
    let ext = match local_form(item, opt) {
        Local::Link(e) | Local::Export(e, _) => e,
    };
    let suffix = format!(".{ext}");
    if item.name.ends_with(&suffix) {
        item.name.clone()
    } else {
        format!("{}{}", item.name, suffix)
    }
}

/// Whether an exported file can go back up (drawings exported as SVG can't).
pub fn can_upload_back(item: &CloudItem) -> bool {
    matches!(item.mime.strip_prefix(GA), Some("document" | "spreadsheet" | "presentation"))
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LinkFile {
    pub url: String,
    pub file_id: String,
    pub account_email: String,
}

pub fn web_url(item: &CloudItem) -> String {
    if let Some(u) = &item.web_link {
        return u.clone();
    }
    let id = item.target_id.as_deref().unwrap_or(&item.id);
    match item.mime.strip_prefix(GA) {
        Some("document") => format!("https://docs.google.com/document/d/{id}/edit"),
        Some("spreadsheet") => format!("https://docs.google.com/spreadsheets/d/{id}/edit"),
        Some("presentation") => format!("https://docs.google.com/presentation/d/{id}/edit"),
        Some("folder") => format!("https://drive.google.com/drive/folders/{id}"),
        _ => format!("https://drive.google.com/open?id={id}"),
    }
}

pub fn link_contents(item: &CloudItem, email: &str) -> String {
    let l = LinkFile { url: web_url(item), file_id: item.target_id.clone().unwrap_or_else(|| item.id.clone()), account_email: email.into() };
    serde_json::to_string(&l).unwrap()
}

pub fn read_link(path: &Path) -> Option<LinkFile> {
    let t = std::fs::read(path).ok()?;
    if t.len() > 64 * 1024 {
        return None;
    }
    serde_json::from_slice(&t).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        let mut it = CloudItem { name: "Budget".into(), mime: format!("{GA}spreadsheet"), id: "x1".into(), ..Default::default() };
        assert_eq!(local_name(&it, ExportOption::Link), "Budget.gdsheet");
        assert_eq!(local_name(&it, ExportOption::MsOffice), "Budget.xlsx");
        assert_eq!(local_name(&it, ExportOption::OpenDocument), "Budget.ods");
        it.mime = format!("{GA}form");
        assert_eq!(local_name(&it, ExportOption::MsOffice), "Budget.gdform");
        assert!(is_link_name("x.gdsheet"));
        assert!(!is_link_name("x.xlsx"));
        let j = link_contents(&CloudItem { id: "abc".into(), mime: format!("{GA}document"), ..Default::default() }, "a@b");
        let l: LinkFile = serde_json::from_str(&j).unwrap();
        assert_eq!(l.file_id, "abc");
        assert!(l.url.contains("/document/d/abc/"));
    }
}
