#[path = "../src/comic.rs"]
mod comic;
#[path = "../src/epub.rs"]
mod epub;

use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
fn stored_and_deflated_comics_list_in_natural_order_and_read_exact_bytes() {
    for name in ["stored.cbz", "deflate.cbz"] {
        let path = fixture(name);
        assert_eq!(comic::comic_pages(&path).unwrap(), vec![
            ("page2.png".to_string(), 12), ("page10.png".to_string(), 11)]);
        assert_eq!(comic::read_zip_entry(&path, "page2.png").unwrap(), b"second image");
        assert_eq!(comic::read_zip_entry(&path, "missing.png"), None);
    }
}

#[test]
fn stored_and_deflated_epubs_read_spine_toc_and_chapter_bytes() {
    for name in ["stored.epub", "deflate.epub"] {
        let path = fixture(name);
        assert_eq!(epub::epub_spine(&path).unwrap(), vec!["one.xhtml", "two.xhtml"]);
        assert_eq!(epub::epub_toc(&path).unwrap(), vec![("First chapter".to_string(), 0)]);
        assert_eq!(epub::read_zip_entry(&path, "one.xhtml").unwrap(), b"<p>Chapter one</p>");
        assert_eq!(epub::read_zip_entry(&path, "missing.xhtml"), None);
    }
}

#[test]
fn invalid_archives_return_none() {
    let path = fixture("invalid.zip");
    assert_eq!(comic::comic_pages(&path), None);
    assert_eq!(comic::read_zip_entry(&path, "page2.png"), None);
    assert_eq!(epub::epub_spine(&path), None);
    assert_eq!(epub::epub_toc(&path), None);
    assert_eq!(epub::epub_cover(&path), None);
    assert_eq!(epub::read_zip_entry(&path, "one.xhtml"), None);
}

/// Names stored as UTF-8 bytes without the UTF-8 flag are listed and read by their UTF-8 spelling.
#[test]
fn unflagged_utf8_names_list_and_read_by_their_utf8_spelling() {
    let path = fixture("legacy-utf8.cbz");
    let pages = comic::comic_pages(&path).unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(comic::read_zip_entry(&path, "é2.png").unwrap(), b"legacy second image");
    assert_eq!(comic::read_zip_entry(&path, &pages[0].0).unwrap(), b"legacy second image");
}

/// Guard saved page indices with UTF-8 bytes whose CP437 ordering would differ.
#[test]
fn unflagged_utf8_order_keeps_the_legacy_physical_page_at_saved_index() {
    let path = fixture("legacy-utf8-order.cbz");
    let pages = comic::comic_pages(&path).unwrap();
    assert_eq!(pages.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), vec!["À.png", "Á.png", "Â.png"]);
    assert_eq!(comic::read_zip_entry(&path,&pages[1].0).unwrap(), b"second physical page");
}

fn boundary_archive(entries: &[(&str,&str,u64)]) -> PathBuf {
    use std::io::Write;
    let path=std::env::temp_dir().join(format!("frayd-epub-boundary-{}.epub",uuid::Uuid::new_v4()));
    let mut archive=zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for (name,prefix,padding) in entries {
        archive.start_file(*name,zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
        archive.write_all(prefix.as_bytes()).unwrap();
        std::io::copy(&mut std::io::Read::take(std::io::repeat(b' '),*padding),&mut archive).unwrap();
    }
    archive.finish().unwrap();path
}

#[test]
fn epub_boundary_rejects_oversized_metadata_declared_root_first_image_and_assets() {
    let container="<container><rootfile full-path=\"book.opf\"/></container>";
    let opf="<package><manifest><item id=\"c\" href=\"declared.png\" properties=\"cover-image\"/></manifest></package>";
    let deep=format!("<package>{}{}</package>","<level>".repeat(128),"</level>".repeat(128));
    let many=format!("<package>{}</package>","<item/>".repeat(100_000));
    for route in ["container","opf","declared","root","first","asset","toc","depth","events"] {
        let mut entries=vec![("META-INF/container.xml",container,0),("book.opf",opf,0)];
        match route {
            "container"=>{entries[0].2=2*1024*1024+1-container.len() as u64;entries.push(("cover.png","small fallback must not mask rejection",0));},
            "opf"=>{entries[1].2=2*1024*1024+1-opf.len() as u64;entries.push(("cover.png","small fallback",0));},
            "declared"=>{entries.push(("declared.png","x",32*1024*1024));entries.push(("cover.png","small fallback",0));},
            "root"=>{entries.push(("cover.png","x",32*1024*1024));entries.push(("zz.png","small fallback",0));},
            "first"=>{entries.push(("first.png","x",32*1024*1024));entries.push(("zz.png","small fallback",0));},
            "asset"=>entries.push(("chapter.xhtml","x",32*1024*1024)),
            "depth"|"events"=>{entries[1].1=if route=="depth" {&deep} else {&many};entries.push(("cover.png","small fallback",0));},
            "toc" => {
                entries[1].1 = "<package><manifest><item id=\"chapter\" href=\"chapter.xhtml\"/><item \
                    id=\"toc\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/></manifest><spine>\
                    <itemref idref=\"chapter\"/></spine></package>";
                entries.push((
                    "toc.ncx",
                    "<ncx><navPoint><navLabel><text>Title</text></navLabel><content src=\"chapter.xhtml\"/></navPoint></ncx>",
                    2 * 1024 * 1024,
                ));
            }
            _ => unreachable!(),
        }
        let path=boundary_archive(&entries);
        match route {
            "asset" => assert!(epub::read_zip_entry(&path, "chapter.xhtml").is_none()),
            "toc" => assert!(epub::epub_toc(&path).is_none()),
            _ => assert!(
                epub::epub_cover(&path).is_none(),
                "{route} oversize must not become fallback artwork"
            ),
        }
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn epub_boundary_real_xml_and_uri_compatibility_preserves_epub2_epub3_and_fallback() {
    let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
    for version in [2,3] {
        let container = "<?xml version='1.0'?><c:container \
            xmlns:c='urn:oasis:names:tc:opendocument:xmlns:container'><c:rootfiles><c:rootfile\n full-path = \
            'OPS/book.opf'/></c:rootfiles></c:container>";
        let opf = format!(
            "<?xml version='1.0'?><!DOCTYPE package SYSTEM 'http://{}/never.dtd'><p:package \
                xmlns:p='http://www.idpf.org/2007/opf'><p:metadata>{}</p:metadata><p:manifest><p:item\n\t id \
                = 'cover' href = 'images/a&amp;b%20%23+%C3%A9%2520.png#fragment' {}/><p:item\n id='chapter' \
                href='chap%20one.xhtml'/><p:item id='toc' href='toc.ncx' \
                media-type='application/x-dtbncx+xml'/></p:manifest><p:spine><p:itemref\n idref = 'chapter'/>\
                </p:spine></p:package>",
            listener.local_addr().unwrap(),
            if version == 2 {
                "<p:meta\n name = 'cover' content = 'cover'/>"
            } else {
                ""
            },
            if version == 3 {
                "properties = 'other cover-image'"
            } else {
                ""
            }
        );
        let path = boundary_archive(&[
            ("META-INF/container.xml", container, 0),
            ("OPS/book.opf", &opf, 0),
            ("000-decoy.png", "wrong fallback", 0),
            ("OPS/images/a&b #+é%20.png", "declared cover", 0),
            ("OPS/chap one.xhtml", "<p>small chapter</p>", 0),
            (
                "OPS/toc.ncx",
                "<!DOCTYPE ncx SYSTEM 'http://127.0.0.1:9/never.dtd'><ncx><navPoint><navLabel><text>Chapter\
                    </text></navLabel><content src=\"chap%20one.xhtml#section\"/></navPoint></ncx>",
                0,
            ),
        ]);
        assert_eq!(epub::epub_cover(&path).unwrap(),b"declared cover");
        assert_eq!(epub::epub_spine(&path).unwrap(),vec!["OPS/chap one.xhtml"]);
        assert_eq!(epub::epub_toc(&path).unwrap(),vec![("Chapter".into(),0)]);
        assert_eq!(epub::read_zip_entry(&path,"OPS/chap one.xhtml").unwrap(),b"<p>small chapter</p>");
        std::fs::remove_file(path).unwrap();
    }
    assert!(
        matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock),
        "DOCTYPE resolution must make no network connection"
    );
    for name in ["cover.png", "images/first.png"] {
        let path = boundary_archive(&[(name, "small image-only fallback", 0)]);
        assert_eq!(
            epub::epub_cover(&path).unwrap(),
            b"small image-only fallback"
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn epub_boundary_manifest_context_ignores_decoys_and_toc_entities_match_spine() {
    let container = "<container><metadata><rootfile full-path=\"wrong.opf\"/></metadata><rootfiles><rootfile \
        full-path=\"OPS/book.opf\"/></rootfiles></container>";
    for format in ["ncx","nav"] {
        let opf = format!(
            "<package><metadata><item id=\"decoy\" href=\"decoy.png\" properties=\"cover-image\"/><meta \
                name=\"cover\" content=\"real\"/></metadata><manifest><item id=\"real\" href=\"real.png\" \
                properties=\"cover-image\"/><item id=\"chapter\" href=\"chapter&amp;one.xhtml\"/><item \
                id=\"toc\" href=\"toc.{format}\" {}/></manifest><spine><itemref idref=\"chapter\"/></spine>\
                <itemref idref=\"decoy\"/></package>",
            if format == "ncx" {
                "media-type=\"application/x-dtbncx+xml\""
            } else {
                "properties=\"nav\""
            }
        );
        let toc = if format == "ncx" {
            "<ncx><navPoint><navLabel><text>Chapter</text></navLabel><content src=\"chapter&amp;one.xhtml#part\"/></navPoint></ncx>"
        } else {
            "<html><nav><a href=\"chapter&amp;one.xhtml#part\">Chapter</a></nav></html>"
        };
        let toc_path = format!("OPS/toc.{format}");
        let path = boundary_archive(&[
            ("META-INF/container.xml", container, 0),
            ("OPS/book.opf", &opf, 0),
            ("OPS/decoy.png", "wrong image", 0),
            ("OPS/real.png", "real declared cover", 0),
            (&toc_path, toc, 0),
            ("OPS/chapter&one.xhtml", "chapter bytes", 0),
        ]);
        assert_eq!(epub::epub_cover(&path).unwrap(),b"real declared cover");
        assert_eq!(epub::epub_spine(&path).unwrap(),vec!["OPS/chapter&one.xhtml"]);
        assert_eq!(epub::epub_toc(&path).unwrap(),vec![("Chapter".into(),0)]);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn epub_boundary_exact_cached_names_keep_literal_percent_without_double_decoding() {
    let path=boundary_archive(&[("chapter one.xhtml","space chapter",0),("literal%20name.xhtml","literal percent chapter",0)]);
    // A percent-encoded cached name is not decoded again, so it does not find the space entry.
    assert_eq!(epub::read_zip_entry(&path,"chapter%20one.xhtml"),None);
    assert_eq!(epub::read_zip_entry(&path,"chapter one.xhtml").unwrap(),b"space chapter");
    // A working literal-percent cached entry must retain its exact identity and order.
    assert_eq!(epub::read_zip_entry(&path,"literal%20name.xhtml").unwrap(),b"literal percent chapter");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn epub_toc_labels_decode_entities_and_collect_only_xml_text() {
    let container = "<container><rootfile full-path=\"OPS/book.opf\"/></container>";
    for format in ["ncx", "nav"] {
        for (label, expected) in [
            ("Q&amp;A", Some("Q&A")),
            ("<span>One</span>", Some("One")),
            (
                " <span title=\"1 > 0\">One</span><!-- ignored --><em> &amp; 二</em> <![CDATA[<b>&amp;</b>]]> &#x1F4D6; &lt;safe&gt; ",
                Some("One & 二 <b>&amp;</b> 📖 <safe>"),
            ),
            ("left <span>middle</span>  right", Some("left middle  right")),
            ("&unknown;", None),
            ("<span>unclosed", None),
        ] {
            let properties = if format == "ncx" { "media-type=\"application/x-dtbncx+xml\"" } else { "properties=\"nav\"" };
            let opf = format!("<package><manifest><item id=\"first\" href=\"first.xhtml\"/><item \
                id=\"chapter\" href=\"chap%20one&amp;two.xhtml\"/><item id=\"toc\" \
                href=\"toc.{format}\" {properties}/></manifest><spine><itemref idref=\"first\"/><itemref \
                idref=\"chapter\"/></spine></package>");
            let toc = if format == "ncx" {
                format!("<ncx><navPoint><navLabel><text>{label}</text></navLabel><content src=\"chap%20one&amp;two.xhtml#part\"/></navPoint></ncx>")
            } else {
                format!("<html><nav><a href=\"chap%20one&amp;two.xhtml#part\">{label}</a></nav></html>")
            };
            let toc_path = format!("OPS/toc.{format}");
            let path = boundary_archive(&[("META-INF/container.xml", container, 0), ("OPS/book.opf", &opf, 0), (&toc_path, &toc, 0)]);
            let actual = epub::epub_toc(&path);
            std::fs::remove_file(path).unwrap();
            assert_eq!(actual, expected.map(|title| vec![(title.to_string(), 1)]), "{format}: {label}");
        }
    }
}

struct CachedZipFixture(PathBuf);
impl Drop for CachedZipFixture {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

/// Construct controlled ZIP metadata, including flags/duplicate names the public writer normalizes.
/// The UTF-8 flag (general-purpose bit 11) is cleared on the first `unflagged` entries.
fn cached_zip_fixture(entries: &[(&str, &str)], unflagged: usize, rename: Option<(&str, &str)>) -> CachedZipFixture {
    use std::io::Write;
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in entries {
        archive.start_file(*name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
        archive.write_all(body.as_bytes()).unwrap();
    }
    let mut bytes = archive.finish().unwrap().into_inner();
    // The flag's high byte sits 7 bytes into a local header and 9 bytes into a central header.
    for (signature, flag_offset) in [(b"PK\x03\x04", 7), (b"PK\x01\x02", 9)] {
        let headers: Vec<usize> = (0..bytes.len().saturating_sub(9)).filter(|&offset| &bytes[offset..offset + 4] == signature).collect();
        for offset in headers.into_iter().take(unflagged) { bytes[offset + flag_offset] &= !0x08; }
    }
    if let Some((from, to)) = rename {
        assert_eq!(from.len(), to.len());
        for offset in 0..=bytes.len() - from.len() {
            if &bytes[offset..offset + from.len()] == from.as_bytes() {
                bytes[offset..offset + to.len()].copy_from_slice(to.as_bytes());
            }
        }
    }
    let path = std::env::temp_dir().join(format!("frayd-cached-zip-{}.cbz", uuid::Uuid::new_v4()));
    std::fs::write(&path, bytes).unwrap();
    CachedZipFixture(path)
}

#[test]
fn cached_zip_page_resolves_legacy_order_subsets_and_flagged_names_without_writes() {
    let fixture = cached_zip_fixture(&[("é2.png", "second"), ("é10.png", "tenth")], 2, None);
    let before = std::fs::read(&fixture.0).unwrap();
    for (names, page, expected) in [
        (vec!["├⌐10.png", "├⌐2.png"], 0, Some("tenth")),
        (vec!["├⌐10.png", "├⌐2.png"], 1, Some("second")),
        (vec!["├⌐10.png"], 0, Some("tenth")),
        (vec!["é2.png"], 0, Some("second")),
        (vec!["missing.png"], 0, None),
        (vec!["├⌐2.png", "├⌐2.png"], 0, None),
    ] {
        let names = names.into_iter().map(String::from).collect::<Vec<_>>();
        assert_eq!(comic::read_cached_page(&fixture.0, &names, page), expected.map(|s| s.as_bytes().to_vec()));
    }
    assert_eq!(std::fs::read(&fixture.0).unwrap(), before);
    let flagged = cached_zip_fixture(&[("é2.png", "flagged")], 0, None);
    assert_eq!(comic::read_cached_page(&flagged.0, &["├⌐2.png".into()], 0), None);
    assert_eq!(comic::read_cached_page(&flagged.0, &["é2.png".into()], 0), Some(b"flagged".to_vec()));
    assert_eq!(comic::read_zip_entry(&flagged.0, "é2.png"), Some(b"flagged".to_vec()));
}

#[test]
fn cached_zip_page_holds_colliding_interpretations_and_duplicate_legacy_archives() {
    for unflagged in [false, true] {
        let collision = cached_zip_fixture(&[("é2.png", "utf8"), ("├⌐2.png", "literal")], if unflagged { 2 } else { 0 }, None);
        let names = ["├⌐2.png".into()];
        if unflagged {
            // Two individually unique interpretations point to different physical entries.
            assert_eq!(comic::read_cached_page(&collision.0, &names, 0), None);
        } else {
            // The flag means both readers interpret the literal name identically.
            assert_eq!(comic::read_cached_page(&collision.0, &names, 0), Some(b"literal".to_vec()));
        }
        assert_eq!(comic::read_cached_page(&collision.0, &["é2.png".into()], 0), Some(b"utf8".to_vec()));
    }
    // Only the first entry loses its flag; the second stays explicitly UTF-8.
    let mixed_fixture = cached_zip_fixture(&[("é2.png", "utf8"), ("├⌐2.png", "literal")], 1, None);
    let canonical: [String; 2] = ["é2.png".into(), "├⌐2.png".into()];
    assert_eq!(comic::read_zip_entry(&mixed_fixture.0, &canonical[0]), Some(b"utf8".to_vec()));
    assert_eq!(comic::read_zip_entry(&mixed_fixture.0, &canonical[1]), Some(b"literal".to_vec()));
    assert_eq!(comic::read_cached_page(&mixed_fixture.0, &canonical, 0), Some(b"utf8".to_vec()));
    assert_eq!(comic::read_cached_page(&mixed_fixture.0, &canonical, 1), Some(b"literal".to_vec()));
    assert_eq!(comic::read_cached_page(&mixed_fixture.0, &["├⌐2.png".into()], 0), None, "legacy label has two targets");
    let duplicate = cached_zip_fixture(&[("é2.png", "first"), ("é3.png", "second")], 2, Some(("é3.png", "é2.png")));
    assert!(zip::ZipArchive::new(std::fs::File::open(&duplicate.0).unwrap()).unwrap().has_duplicate_names());
    assert_eq!(comic::read_cached_page(&duplicate.0, &["├⌐2.png".into()], 0), None);
    let unrelated = cached_zip_fixture(&[("é2.png", "image"), ("note-a.txt", "one"), ("note-b.txt", "two")], 3, Some(("note-b.txt", "note-a.txt")));
    assert_eq!(comic::read_cached_page(&unrelated.0, &["é2.png".into()], 0), Some(b"image".to_vec()));
    assert_eq!(comic::read_cached_page(&unrelated.0, &["├⌐2.png".into()], 0), None, "legacy recovery conservatively holds on duplicate archives");
}
