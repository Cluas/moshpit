mod common;
use common::*;

#[test]
fn probe() {
    // DECAWM off, a wide char at the last column.
    let mut t = term(5, 3, 100);
    t.feed("\x1b[?7labcd你".as_bytes());
    eprintln!("nowrap wide at edge: {:?} {:?} cursor {:?}", t.row_text(0), t.row_text(1), { let f = t.frame(); (f.cursor.col, f.cursor.row) });
    // VS16 emoji at the edge with wrap.
    let mut t = term(5, 3, 100);
    t.feed("abcd\u{2764}\u{fe0f}x".as_bytes());
    eprintln!("vs16 at edge: {:?} {:?} {:?}", t.row_text(0), t.row_text(1), { let f = t.frame(); (f.cursor.col, f.cursor.row) });
    // A wide char overwritten half by a narrow one.
    let mut t = term(10, 3, 100);
    t.feed("你好\x1b[1;2Hx".as_bytes());
    eprintln!("half overwrite: {:?}", t.row_text(0));
    t.feed("\x1b[1;1Hy".as_bytes());
    eprintln!("first half overwrite: {:?}", t.row_text(0));
    // Reflow of a line ending with a wide char at a width that splits it.
    let mut t = term(6, 4, 100);
    t.feed("ab你好cd".as_bytes());
    eprintln!("before: {:?} {:?}", t.row_text(0), t.row_text(1));
    t.resize(3, 4);
    eprintln!("at 3: {:?} {:?} {:?} {:?}", t.row_text(0), t.row_text(1), t.row_text(2), t.row_text(3));
    t.resize(6, 4);
    eprintln!("back at 6: {:?} {:?}", t.row_text(0), t.row_text(1));
    // Scrolled back, then a program enters and leaves the alternate screen.
    let mut t = term(10, 3, 100);
    for i in 0..20 {
        t.feed(format!("line{i}\r\n").as_bytes());
    }
    t.scroll(5);
    let off = t.display_offset();
    t.feed(b"\x1b[?1049hALT\x1b[?1049l");
    eprintln!("offset before {off} after alt {}", t.display_offset());
    // Scrolled back, a tap: what row does the mouse report say?
    t.feed(b"\x1b[?1000h\x1b[?1006h");
    let m = t.mouse(0, 0, 1, 1, 0);
    eprintln!("tap while scrolled back: {:?} offset now {}", String::from_utf8_lossy(&m), t.display_offset());
}
