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
    assert_eq!(epub::read_zip_entry(&path, "one.xhtml"), None);
}
