//! The 8-bit encodings older formats leave their text in, named the ways
//! those formats name them: a Windows code page number (RTF's `\ansicpg`,
//! an Outlook message's code page property), a font's character set (Word
//! 6/95 and RTF font tables), or a locale (an Outlook message's language).

use encoding_rs::*;

/// The encoding of Windows code page `number`, when it is one text is
/// written in and `encoding_rs` knows it.
pub fn by_number(number: u32) -> Option<&'static Encoding> {
    Some(match number {
        866 => IBM866,
        874 => WINDOWS_874,
        932 => SHIFT_JIS,
        936 => GBK,
        949 => EUC_KR,
        950 => BIG5,
        1200 => UTF_16LE,
        1201 => UTF_16BE,
        1250 => WINDOWS_1250,
        1251 => WINDOWS_1251,
        1252 | 28591 => WINDOWS_1252,
        1253 => WINDOWS_1253,
        1254 => WINDOWS_1254,
        1255 => WINDOWS_1255,
        1256 => WINDOWS_1256,
        1257 => WINDOWS_1257,
        1258 => WINDOWS_1258,
        10000 => MACINTOSH,
        10007 => X_MAC_CYRILLIC,
        20866 => KOI8_R,
        21866 => KOI8_U,
        28592 => ISO_8859_2,
        28593 => ISO_8859_3,
        28594 => ISO_8859_4,
        28595 => ISO_8859_5,
        28596 => ISO_8859_6,
        28597 => ISO_8859_7,
        28598 => ISO_8859_8,
        28600 => ISO_8859_10,
        28603 => ISO_8859_13,
        28604 => ISO_8859_14,
        28605 => ISO_8859_15,
        28606 => ISO_8859_16,
        50220..=50222 => ISO_2022_JP,
        51932 => EUC_JP,
        54936 => GB18030,
        65001 => UTF_8,
        _ => return None,
    })
}

/// The encoding a font's character set implies — `None` for ANSI, the
/// default and Symbol, which say nothing beyond the document's own code
/// page, and for OEM and Mac sets, which body text does not use.
pub fn by_charset(charset: u8) -> Option<&'static Encoding> {
    match charset {
        128 => Some(SHIFT_JIS),
        129 => Some(EUC_KR),
        134 => Some(GBK),
        136 => Some(BIG5),
        161 => Some(WINDOWS_1253),
        162 => Some(WINDOWS_1254),
        163 => Some(WINDOWS_1258),
        177 => Some(WINDOWS_1255),
        178 => Some(WINDOWS_1256),
        186 => Some(WINDOWS_1257),
        204 => Some(WINDOWS_1251),
        222 => Some(WINDOWS_874),
        238 => Some(WINDOWS_1250),
        _ => None,
    }
}

/// The ANSI code page Windows uses for locale `lcid`: Windows-1252 unless
/// its language is written in another script.
pub fn by_locale(lcid: u32) -> &'static Encoding {
    match lcid {
        // Chinese: simplified in the PRC and Singapore, traditional elsewhere.
        0x0804 | 0x1004 => return GBK,
        0x0404 | 0x0C04 | 0x1404 => return BIG5,
        // Serbian and Bosnian in Cyrillic; in Latin they are Central European.
        0x0C1A | 0x1C1A | 0x201A => return WINDOWS_1251,
        _ => {}
    }
    match lcid & 0x3FF {
        0x02 | 0x19 | 0x22 | 0x23 | 0x2F | 0x3F | 0x40 | 0x44 | 0x50 => WINDOWS_1251,
        0x05 | 0x0E | 0x15 | 0x18 | 0x1A | 0x1B | 0x1C | 0x24 => WINDOWS_1250,
        0x08 => WINDOWS_1253,
        0x1F | 0x2C => WINDOWS_1254,
        0x0D => WINDOWS_1255,
        0x01 | 0x20 | 0x29 => WINDOWS_1256,
        0x25..=0x27 => WINDOWS_1257,
        0x2A => WINDOWS_1258,
        0x1E => WINDOWS_874,
        0x11 => SHIFT_JIS,
        0x12 => EUC_KR,
        _ => WINDOWS_1252,
    }
}
