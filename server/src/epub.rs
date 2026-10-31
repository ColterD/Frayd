use std::io::Read;
use std::path::Path;

const METADATA_LIMIT: u64 = 2 * 1024 * 1024;
const BINARY_LIMIT: u64 = 32 * 1024 * 1024;
const XML_EVENT_LIMIT: usize = 100_000;
const XML_DEPTH_LIMIT: usize = 128;
const CONTAINER_NAMESPACE: &str = "urn:oasis:names:tc:opendocument:xmlns:container";
const OPF_NAMESPACE: &str = "http://www.idpf.org/2007/opf";

#[derive(Debug, PartialEq)]
enum ReadRejection { Io, TooLarge, InvalidXml }
type EntryRead<T> = Result<Option<T>, ReadRejection>;

/// Bound bytes actually produced by the decompressor, independently of ZIP size declarations.
fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>,ReadRejection> {
    let mut bytes=Vec::new();
    reader.take(limit.checked_add(1).ok_or(ReadRejection::TooLarge)?).read_to_end(&mut bytes).map_err(|_|ReadRejection::Io)?;
    if bytes.len() as u64 > limit {Err(ReadRejection::TooLarge)}else{Ok(bytes)}
}

fn read_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str, limit: u64) -> EntryRead<Vec<u8>> {
    match zip.by_name(name) {
        Ok(entry)=>read_limited(entry,limit).map(Some),
        Err(zip::result::ZipError::FileNotFound)=>Ok(None),
        Err(_)=>Err(ReadRejection::Io),
    }
}

fn read_text(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> EntryRead<String> {
    Ok(read_entry(zip,name,METADATA_LIMIT)?.and_then(|bytes|String::from_utf8(bytes).ok()))
}

/// Strip the URI fragment before decoding once; decoded percent signs and literal plus signs survive.
fn resolve_href(base: &str, href: &str) -> Option<String> {
    let decoded=urlencoding::decode(href.split('#').next()?).ok()?;
    if decoded.starts_with('/') {return None;}
    let joined=match base.rsplit_once('/') {Some((directory,_))=>format!("{directory}/{decoded}"),None=>decoded.into_owned()};
    let mut parts=Vec::new();
    for part in joined.split('/') {
        match part {""|"."=>(),".."=>{parts.pop()?;},value=>parts.push(value)}
    }
    if parts.is_empty() {None}else{Some(parts.join("/"))}
}

struct XmlParent { name:String, unbound:bool }

/// Parse structural attributes with bounded parent context. DTD declarations are never fetched or expanded.
fn xml_structure(
    text: &str,
    namespace: &str,
    mut visit: impl FnMut(&str, &[XmlParent], bool, &std::collections::HashMap<String, String>),
) -> Result<(), ReadRejection> {
    use quick_xml::{events::Event,name::ResolveResult};
    let mut reader=quick_xml::reader::NsReader::from_str(text);
    reader.config_mut().check_end_names=true;
    let mut version=quick_xml::XmlVersion::Implicit1_0;
    let mut parents:Vec<XmlParent>=Vec::new();
    let mut roots=0usize;
    for _ in 0..XML_EVENT_LIMIT {
        let event=reader.read_event().map_err(|_|ReadRejection::InvalidXml)?;
        match event {
            Event::Start(ref element)|Event::Empty(ref element)=>{
                if parents.is_empty() {roots+=1;if roots>1 {return Err(ReadRejection::InvalidXml);}}
                if parents.len()+1 > XML_DEPTH_LIMIT {return Err(ReadRejection::TooLarge);}
                let local=element.local_name();let name=local.as_ref();
                let (resolved,_)=reader.resolver().resolve_element(element.name());
                let unbound=matches!(resolved,ResolveResult::Unbound);
                if parents.is_empty()
                    && name
                        != if namespace == CONTAINER_NAMESPACE {
                            "container"
                        } else {
                            "package"
                        }
                {
                    return Err(ReadRejection::InvalidXml);
                }
                if parents.is_empty() || matches!(name,"rootfile"|"rootfiles"|"item"|"itemref"|"meta"|"manifest"|"spine"|"metadata") {
                    match resolved {
                        ResolveResult::Unbound => (),
                        ResolveResult::Bound(value) if value.as_ref() == namespace => (),
                        _ => return Err(ReadRejection::InvalidXml),
                    }
                    let mut attributes=std::collections::HashMap::new();
                    for attribute in element.attributes() {
                        let attribute=attribute.map_err(|_|ReadRejection::InvalidXml)?;
                        let key=attribute.key.as_ref();
                        if matches!(key,"full-path"|"id"|"href"|"properties"|"media-type"|"idref"|"name"|"content") {
                            attributes.insert(
                                key.to_string(),
                                attribute
                                    .normalized_value(version)
                                    .map_err(|_| ReadRejection::InvalidXml)?
                                    .into_owned(),
                            );
                        }
                    }
                    visit(name,&parents,unbound,&attributes);
                }
                if matches!(event,Event::Start(_)) {parents.push(XmlParent{name:name.to_string(),unbound});}
            }
            Event::Decl(declaration)=>{version=declaration.xml_version().map_err(|_|ReadRejection::InvalidXml)?;},
            Event::End(_)=>{parents.pop().ok_or(ReadRejection::InvalidXml)?;}
            Event::Eof=>return if parents.is_empty() && roots==1 {Ok(())}else{Err(ReadRejection::InvalidXml)},
            _=>(),
        }
    }
    Err(ReadRejection::TooLarge)
}

fn parent_context(parents:&[XmlParent],expected:&[&str])->bool {
    parents.iter().map(|parent|parent.name.as_str()).eq(expected.iter().copied())
}

struct ManifestItem { id:String, href:String, properties:String, media_type:String }
struct Package { path:String, manifest:Vec<ManifestItem>, spine:Vec<String>, cover_id:Option<String> }

fn package(zip: &mut zip::ZipArchive<std::fs::File>) -> EntryRead<Package> {
    let Some(container)=read_text(zip,"META-INF/container.xml")? else {return Ok(None);};
    let mut opf_path=None;
    let parsed=xml_structure(&container,CONTAINER_NAMESPACE,|name,parents,unbound,attributes| {
        let legacy_direct=unbound && parents.len()==1 && parents[0].unbound && parents[0].name=="container";
        if name == "rootfile"
            && opf_path.is_none()
            && (parent_context(parents, &["container", "rootfiles"]) || legacy_direct)
        {
            opf_path = attributes.get("full-path").cloned();
        }
    });
    if matches!(parsed,Err(ReadRejection::InvalidXml)) {return Ok(None);}parsed?;
    let Some(path)=opf_path.and_then(|path|resolve_href("",&path)) else {return Ok(None);};
    let Some(opf)=read_text(zip,&path)? else {return Ok(None);};
    let mut result=Package{path,manifest:Vec::new(),spine:Vec::new(),cover_id:None};
    let parsed=xml_structure(&opf,OPF_NAMESPACE,|name,parents,_,attributes| {
        match name {
            "item" if parent_context(parents, &["package", "manifest"]) => {
                if let (Some(id), Some(href)) = (attributes.get("id"), attributes.get("href")) {
                    result.manifest.push(ManifestItem {
                        id: id.clone(),
                        href: href.clone(),
                        properties: attributes.get("properties").cloned().unwrap_or_default(),
                        media_type: attributes.get("media-type").cloned().unwrap_or_default(),
                    });
                }
            }
            "itemref" if parent_context(parents, &["package", "spine"]) => {
                if let Some(id) = attributes.get("idref") {
                    result.spine.push(id.clone());
                }
            }
            "meta"
                if parent_context(parents, &["package", "metadata"])
                    && attributes.get("name").is_some_and(|name| name == "cover") =>
            {
                result.cover_id = attributes.get("content").cloned();
            }
            _ => (),
        }
    });
    if matches!(parsed,Err(ReadRejection::InvalidXml)) {return Ok(None);}parsed?;
    Ok(Some(result))
}

fn package_spine(package: &Package) -> Vec<String> {
    let mut hrefs=std::collections::HashMap::new();
    for item in &package.manifest {hrefs.entry(item.id.as_str()).or_insert(item.href.as_str());}
    package.spine.iter().filter_map(|id|hrefs.get(id.as_str()))
        .filter_map(|href|resolve_href(&package.path,href)).collect()
}

/// Return decoded, ZIP-root-bounded manifest names in reading order.
pub fn epub_spine(path: &Path) -> Option<Vec<String>> {
    let mut zip=crate::comic::open_zip(path)?;
    let spine=package_spine(&package(&mut zip).ok()??);
    if spine.is_empty() {None}else{Some(spine)}
}

/// Missing declarations may fall back; resource rejection aborts the complete artwork selection.
fn cover(zip: &mut zip::ZipArchive<std::fs::File>, path: &Path) -> EntryRead<Vec<u8>> {
    if let Some(package)=package(zip)? {
        let declared=package.manifest.iter().find(|item|item.properties.split_whitespace().any(|token|token=="cover-image")
            || package.cover_id.as_ref()==Some(&item.id));
        if let Some(name)=declared.and_then(|item|resolve_href(&package.path,&item.href)) {
            if let Some(bytes)=read_entry(zip,&name,BINARY_LIMIT)? {return Ok(Some(bytes));}
        }
    }
    for name in ["cover.jpeg","cover.jpg","cover.png"] {
        if let Some(bytes)=read_entry(zip,name,BINARY_LIMIT)? {return Ok(Some(bytes));}
    }
    let first=crate::comic::comic_pages(path).and_then(|pages|pages.first().map(|page|page.0.clone()));
    match first {Some(name)=>read_entry(zip,&name,BINARY_LIMIT),None=>Ok(None)}
}

pub fn epub_cover(path: &Path) -> Option<Vec<u8>> {
    let mut zip=crate::comic::open_zip(path)?;
    cover(&mut zip,path).ok().flatten()
}

/// Table of contents as (title, spine index) pairs, from the NCX when declared, else the EPUB 3 nav.
pub fn epub_toc(path: &Path) -> Option<Vec<(String, usize)>> {
    let mut zip=crate::comic::open_zip(path)?;
    let package=package(&mut zip).ok()??;
    let opf_path=&package.path;
    let spine_files=package_spine(&package);
    let spine_idx=|href:&str|spine_files.iter().position(|file|file==href);
    let ncx_href=package.manifest.iter().find(|item|item.media_type=="application/x-dtbncx+xml").map(|item|item.href.clone());
    let nav_href=package.manifest.iter().find(|item|item.properties.split_whitespace().any(|token|token=="nav")).map(|item|item.href.clone());
    let toc_src = ncx_href.clone().or(nav_href)?; // ncx preferred, nav fallback
    let toc_full=resolve_href(opf_path,&toc_src)?;
    let text = read_text(&mut zip, &toc_full).ok()??;
    let mut toc: Vec<(String, usize)> = Vec::new();
    if ncx_href.is_some() {
        // navPoint: <navLabel><text>Title</text></navLabel><content src="..."/>
        for np in text.split("<navPoint").skip(1) {
            let title = between(np, "<text>", "</text>")
                .and_then(|title| toc_title_text(&title))
                .unwrap_or_default();
            let src = match attr(np,"src") {Some(src)=>src,None=>continue};
            let Some(full)=resolve_href(&toc_full,&src) else {continue;};
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
            let title = between(a, ">", "</a>")
                .and_then(|title| toc_title_text(&title))
                .unwrap_or_default();
            let Some(full)=resolve_href(&toc_full,&href) else {continue;};
            if !title.is_empty() {
                if let Some(idx) = spine_idx(&full) {
                    toc.push((title, idx));
                }
            }
        }
    }
    if toc.is_empty() { None } else { Some(toc) }
}

/// Read an exact already-resolved ZIP name for HTTP chapter/asset consumers; never decode it twice.
pub fn read_zip_entry(path: &Path, name: &str) -> Option<Vec<u8>> {
    let mut zip=crate::comic::open_zip(path)?;
    read_entry(&mut zip,name,BINARY_LIMIT).ok().flatten()
}

/// Collect bounded XML character data without turning decoded text back into markup.
/// Only predefined and numeric references resolve; DTD/custom entities never expand.
fn toc_title_text(markup: &str) -> Option<String> {
    use quick_xml::events::Event;
    if markup.len() as u64 > METADATA_LIMIT { return None; }
    let mut reader = quick_xml::Reader::from_str(markup);
    reader.config_mut().check_end_names = true;
    let mut depth = 0usize;
    let mut title = String::new();
    for _ in 0..XML_EVENT_LIMIT {
        match reader.read_event().ok()? {
            Event::Start(_) => {
                depth += 1;
                if depth > XML_DEPTH_LIMIT { return None; }
            }
            Event::Empty(_) => {
                if depth >= XML_DEPTH_LIMIT { return None; }
            }
            Event::End(_) => { depth = depth.checked_sub(1)?; }
            Event::Text(text) => title.push_str(&text.xml10_content()),
            Event::CData(text) => title.push_str(&text.xml10_content()),
            Event::GeneralRef(reference) => {
                if let Some(character) = reference.resolve_char_ref().ok()? {
                    title.push(character);
                } else {
                    title.push_str(quick_xml::escape::resolve_predefined_entity(reference.as_ref())?);
                }
            }
            Event::DocType(_) | Event::Decl(_) => return None,
            Event::Eof => return if depth == 0 { Some(title.trim().to_string()) } else { None },
            _ => (),
        }
    }
    None
}

fn attr(s: &str, name: &str) -> Option<String> {
    let pat = format!("{name}=\"");
    let start = s.find(&pat)? + pat.len();
    let end = s[start..].find('"')? + start;
    normalized_attribute(&s[start..end])
}

fn normalized_attribute(value:&str)->Option<String> {
    let attribute=quick_xml::events::attributes::Attribute{key:quick_xml::name::QName("value"),value:std::borrow::Cow::Borrowed(value)};
    attribute.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|value|value.into_owned())
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
    fn toc_title_text_rejects_resource_overflow_and_entity_declarations() {
        assert_eq!(toc_title_text("&amp;lt;safe&amp;gt;"), Some("&lt;safe&gt;".into()));
        assert_eq!(toc_title_text("<!DOCTYPE span [<!ENTITY custom 'expanded'>]><span>&custom;</span>"), None);
        assert_eq!(toc_title_text(&"x".repeat(METADATA_LIMIT as usize + 1)), None);
        let deep = format!("{}{}", "<span>".repeat(XML_DEPTH_LIMIT + 1), "</span>".repeat(XML_DEPTH_LIMIT + 1));
        assert_eq!(toc_title_text(&deep), None);
        assert_eq!(toc_title_text(&"<br/>".repeat(XML_EVENT_LIMIT)), None);
    }

    #[test]
    fn package_spine_preserves_order_skips_missing_and_keeps_first_duplicate() {
        let item=|id:&str,href:&str|ManifestItem{id:id.into(),href:href.into(),properties:String::new(),media_type:String::new()};
        let package = Package {
            path: "OPS/book.opf".into(),
            manifest: vec![
                item("first", "first.xhtml"),
                item("duplicate", "chosen.xhtml"),
                item("duplicate", "ignored.xhtml"),
            ],
            spine: vec![
                "duplicate".into(),
                "missing".into(),
                "first".into(),
                "duplicate".into(),
            ],
            cover_id: None,
        };
        assert_eq!(package_spine(&package),vec!["OPS/chosen.xhtml","OPS/first.xhtml","OPS/chosen.xhtml"]);
    }

    #[test]
    fn bounded_stream_ignores_declared_size_and_rejects_error_or_overflow() {
        use std::cell::Cell;
        struct Underreported<'a> {remaining:u64, consumed:&'a Cell<u64>}
        impl Read for Underreported<'_> {
            fn read(&mut self,buf:&mut [u8])->std::io::Result<usize> {
                let length=(buf.len() as u64).min(self.remaining) as usize;buf[..length].fill(b'x');
                self.remaining-=length as u64;self.consumed.set(self.consumed.get()+length as u64);Ok(length)
            }
        }
        for limit in [METADATA_LIMIT,BINARY_LIMIT] {
            assert_eq!(read_limited(std::io::repeat(b'x').take(limit),limit).unwrap().len() as u64,limit);
            let consumed=Cell::new(0);
            let reader=Underreported{remaining:limit+20,consumed:&consumed};
            assert_eq!(read_limited(reader,limit),Err(ReadRejection::TooLarge));assert_eq!(consumed.get(),limit+1);
        }
        struct Failing;
        impl Read for Failing {fn read(&mut self,_:&mut [u8])->std::io::Result<usize>{Err(std::io::Error::other("read rejected"))}}
        assert_eq!(read_limited(Failing,10),Err(ReadRejection::Io));
        assert_eq!(read_limited(std::io::empty(),u64::MAX),Err(ReadRejection::TooLarge));
    }

    #[test]
    fn epub_uri_decodes_once_without_changing_plus_fragment_or_zip_root() {
        for (href, expected) in [
            (
                "images/front%20cover.png#page",
                "OPS/images/front cover.png",
            ),
            ("images/a%23b+%C3%A9.png#ignored", "OPS/images/a#b+é.png"),
            ("../cover%2520.png", "cover%20.png"),
            ("./images/../cover.png", "OPS/cover.png"),
        ] {
            assert_eq!(resolve_href("OPS/book.opf",href).as_deref(),Some(expected));
        }
        for href in ["../../escape.png","%2e%2e/%2e%2e/escape.png","/absolute.png","%FF.png"] {assert_eq!(resolve_href("OPS/book.opf",href),None);}
    }

    #[test]
    fn xml_structure_enforces_namespaces_entities_depth_events_and_end_names() {
        let parse=|text:&str|xml_structure(text,OPF_NAMESPACE,|_,_,_,_|{});
        assert!(parse("<?xml version='1.0'?><!DOCTYPE package SYSTEM 'http://127.0.0.1:9/never-fetch'>\
            <package xmlns='http://www.idpf.org/2007/opf'><item id='cover' href='a&amp;b&#32;c.png'/>\
            </package>").is_ok());
        for invalid in [
            "<package><item href='&custom;'/></package>",
            "<!DOCTYPE package [<!ENTITY custom 'expanded'>]><package><item href='&custom;'/></package>",
            "<package><item></package>",
            "<package xmlns='urn:wrong'><item/></package>",
            "<package><wrong:item xmlns:wrong='urn:wrong'/></package>",
        ] {
            assert!(parse(invalid).is_err(), "{invalid}");
        }
        let deep = format!(
            "<package>{}{}</package>",
            "<level>".repeat(XML_DEPTH_LIMIT),
            "</level>".repeat(XML_DEPTH_LIMIT)
        );
        assert!(parse(&deep) == Err(ReadRejection::TooLarge));
        let many=format!("<package>{}</package>","<item/>".repeat(XML_EVENT_LIMIT));assert!(parse(&many)==Err(ReadRejection::TooLarge));
    }

    #[test]
    fn attr_parse() {
        let s = "<item id=\"c1\" href=\"ch1.xhtml\" media-type=\"x\"/>";
        assert_eq!(attr(s, "id").unwrap(), "c1");
        assert_eq!(attr(s, "href").unwrap(), "ch1.xhtml");
        assert_eq!(attr(s, "missing"), None);
    }
}
