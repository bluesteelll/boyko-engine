//! Render wave PX0 (`px0/cut.md` §1.3 #3) — **the image-dump WRITER's format table equals the
//! READER's**: `present::image_dump_texel_bytes` (what the recorder copies) against
//! `boyko_shaderdsl::pixfmt::texel_bytes` (what every reader decodes — `post_eval`, the dump
//! helpers, `tools/golden_diff.py`'s pinned mirror).
//!
//! A format the writer copies but no reader decodes is a dump nobody can read; a format the reader
//! knows at a different size is a dump everybody misreads. Both directions are checked over every
//! VkFormat code the core enum spans, so a code added to one table alone is red here.

use boyko_rhi_vulkan::present::image_dump_texel_bytes;
use boyko_shaderdsl::pixfmt::{DUMP_FORMATS, FORMAT_BUFFER, texel_bytes};

/// The highest core `VkFormat` value (`VK_FORMAT_ASTC_12x12_SRGB_BLOCK` = 184).
const MAX_CORE_FORMAT: i32 = 184;

#[test]
fn every_written_format_is_read_at_the_same_size() {
    let mut written = 0;
    for code in 1..=MAX_CORE_FORMAT {
        if let Some(w) = image_dump_texel_bytes(code) {
            written += 1;
            let r = texel_bytes(code as u32);
            assert_eq!(r, Some(w), "VkFormat {code}: the writer copies {w} B/texel, the reader table says {r:?}");
        }
    }
    assert!(written >= 9, "the writer table names {written} formats; PX0's has nine");
}

#[test]
fn every_read_image_format_is_one_the_writer_copies() {
    for code in DUMP_FORMATS {
        if code == FORMAT_BUFFER {
            // A buffer dump (format 0) has no image copy: its writers are PX3 / V10's, and the
            // reader's contract for it is `boyko_shaderdsl::dump`'s.
            continue;
        }
        assert_eq!(
            image_dump_texel_bytes(code as i32),
            texel_bytes(code),
            "VkFormat {code}: the reader decodes it but the writer table disagrees"
        );
    }
}

/// `lit`'s HDR candidate is in both tables (the format PX1a dumps).
#[test]
fn b10g11r11_is_in_both_tables() {
    assert_eq!(image_dump_texel_bytes(122), Some(4));
    assert_eq!(texel_bytes(122), Some(4));
}
