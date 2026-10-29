use std::path::Path;

/// Minimal epub spine extraction: container.xml → OPF path → spine idrefs → hrefs.
/// Returns manifest hrefs in reading order (relative to OPF dir).
pub fn epub_spine(path: &Path) -> Option<Vec<String>> {
    let f = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(f).ok()?;

    // 1. container.xml → rootfile full-path
    let container = read(&mut zip, "META-INF/container.xml")?;
    let opf_path = between(&container, "full-path=\"", "\"")?;

    // 2. OPF: manifest (id→href) + spine (idref order)
    let opf = read(&mut zip, &opf_path)?;
    let opf_dir = opf_path.rsplit_once('/').map(|(d, _)| d);

    let mut hrefs = std::collections::HashMap::new();
    for m in opf.split("<item ").skip(1) {
        if let (Some(id), Some(href)) = (attr(m, "id"), attr(m, "href")) {
            hrefs.insert(id, href);
        }
    }
    let mut spine = Vec::new();
    for ir in opf.split("<itemref ").skip(1) {
        if let Some(idref) = attr(ir, "idref") {
            if let Some(href) = hrefs.get(&idref) {
                let full = match opf_dir {
                    Some(d) => format!("{d}/{href}"),
                    None => href.clone(),
                };
                spine.push(full);
            }
        }
    }
    if spine.is_empty() { None } else { Some(spine) }
}

/// TOC entries: (title, spine index). From toc.ncx (epub2) or nav.xhtml (epub3).
/// Resolved via spine hrefs → first spine chunk containing the target file.
pub fn epub_toc(path: &Path) -> Option<Vec<(String, usize)>> {
    let f = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(f).ok()?;
    let container = read(&mut zip, "META-INF/container.xml")?;
    let opf_path = between(&container, "full-path=\"", "\"")?;
    let opf = read(&mut zip, &opf_path)?;
    let opf_dir = opf_path.rsplit_once('/').map(|(d, _)| d);
    let resolve = |href: &str| -> String {
        // strip #fragment, url-decode minimal
        let base = href.split('#').next().unwrap_or(href);
        match opf_dir {
            Some(d) => {
                // OPF dir may itself need resolving (../ in href)
                let joined = format!("{d}/{base}");
                let mut parts: Vec<&str> = Vec::new();
                for p in joined.split('/') {
                    match p {
                        "." => {}
                        ".." => { parts.pop(); }
                        s => parts.push(s),
                    }
                }
                parts.join("/")
            }
            None => base.to_string(),
        }
    };

    let spine = epub_spine(path)?;
    let spine_files: Vec<String> = spine.iter().map(|h| resolve(h)).collect();
    let spine_idx = |href: &str| -> Option<usize> {
        let r = resolve(href);
        spine_files.iter().position(|f| *f == r)
    };

    // epub3: nav.xhtml — find <item properties~="nav"> in manifest
    let nav_href = opf.split("<item ")
        .find(|m| attr(m, "properties").map(|p| p.split_whitespace().any(|t| t == "nav")).unwrap_or(false))
        .and_then(|m| attr(m, "href"));
    // epub2: toc.ncx — path from spine <itemref> or manifest item id="ncx"
    let ncx_href = opf.split("<item ")
        .find(|m| attr(m, "media-type").map(|t| t == "application/x-dtbncx+xml").unwrap_or(false))
        .and_then(|m| attr(m, "href"));

    let toc_src = ncx_href.clone().or(nav_href)?; // ncx preferred, nav fallback
    let toc_full = match opf_dir {
        Some(d) => format!("{d}/{toc_src}"),
        None => toc_src.clone(),
    };
    // TOC srcs resolve relative to the TOC file's dir; but that dir is relative to OPF dir.
    // TOC hrefs in the OPF manifest are OPF-relative, so: opf_dir + toc_dir + src.
    let base_dir: Option<String> = match (opf_dir, toc_src.rsplit_once('/')) {
        (Some(d), Some((td, _))) => Some(format!("{d}/{td}")),
        (Some(d), None) => Some(d.to_string()),
        (None, Some((td, _))) => Some(td.to_string()),
        (None, None) => None,
    };
    let raw = read(&mut zip, &toc_full)?;

    let text = raw;
    let mut toc: Vec<(String, usize)> = Vec::new();
    if ncx_href.is_some() {
        // navPoint: <navLabel><text>Title</text></navLabel><content src="..."/>
        for np in text.split("<navPoint").skip(1) {
            let title = between(np, "<text>", "</text>")
                .map(|t| t.trim().to_string())
                .unwrap_or_default();
            let src = match between(np, "src=\"", "\"") { Some(s) => s, None => continue };
            let full = match &base_dir {
                Some(d) => format!("{d}/{src}"),
                None => src.clone(),
            };
            if !title.is_empty() {
                if let Some(idx) = spine_idx(&full) {
                    toc.push((title, idx));
                }
            }
        }
    } else {
        // nav.xhtml: <a href="...">Title</a> inside <nav>
        for a in text.split("<a ").skip(1) {
            let href = match attr(a, "href") { Some(h) => h, None => continue };
            let title = between(&format!(">{a}"), ">", "</a>")
                .map(|t| t.trim().to_string())
                .unwrap_or_default();
            let full = match &base_dir {
                Some(d) => format!("{d}/{href}"),
                None => href.clone(),
            };
            if !title.is_empty() {
                if let Some(idx) = spine_idx(&full) {
                    toc.push((title, idx));
                }
            }
        }
    }
    if toc.is_empty() { None } else { Some(toc) }
}

/// Read one entry from a zip archive by exact name.
pub fn read_zip_entry(path: &Path, name: &str) -> Option<Vec<u8>> {
    let f = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(f).ok()?;
    let mut entry = zip.by_name(name).ok()?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    std::io::Read::read_to_end(&mut entry, &mut buf).ok()?;
    Some(buf)
}

fn read(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut e = zip.by_name(name).ok()?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut e, &mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

fn attr(s: &str, name: &str) -> Option<String> {
    let pat = format!("{name}=\"");
    let start = s.find(&pat)? + pat.len();
    let end = s[start..].find('"')? + start;
    Some(s[start..end].to_string())
}

fn between(s: &str, open: &str, close: &str) -> Option<String> {
    let start = s.find(open)? + open.len();
    let end = s[start..].find(close)? + start;
    Some(s[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attr_parse() {
        let s = "<item id=\"c1\" href=\"ch1.xhtml\" media-type=\"x\"/>";
        assert_eq!(attr(s, "id").unwrap(), "c1");
        assert_eq!(attr(s, "href").unwrap(), "ch1.xhtml");
        assert_eq!(attr(s, "missing"), None);
    }
}
