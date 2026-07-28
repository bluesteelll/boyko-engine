//! The virtual-geometry campaign's freeze tripwire: `docs/VG-CAMPAIGN-THRESHOLDS.toml` must hash
//! to the value recorded here, and this test does nothing else.
//!
//! # Why this file exists, and why it is in this crate
//!
//! The campaign's thresholds file is author-frozen: it carries K1's decision rule, the census
//! resolution ladder and the one pre-registered tolerance a rung reads, all authored before any
//! measurement is reachable, so that no threshold can be chosen by whoever measures against it. A
//! freeze is only worth its name if something re-asserts it.
//!
//! Nothing did. The history is short and it is the whole argument for this file:
//!
//! * **Rev 2** recorded a sha256 over a file whose schedule *required* it to change — a legitimate
//!   fill was guaranteed to break the hash before the first rung that asserted it, and once
//!   "re-record the hash" is routine the tripwire carries no signal at all.
//! * **Rev 4** avoided that and produced its mirror image: **guaranteed not to fire.** The four
//!   rungs named as re-asserting the hash were every one of them a skipped or `#[ignore]`d
//!   GPU/corpus test on a box whose CI never exercises the GPU path.
//! * **Rev 6** added `[hash_assertion].must_run_in_plain_workspace_test = true` to fix exactly
//!   that — and gave it no rung and no file, so the field described an intention nobody
//!   implements.
//! * **Rev 7** named this path in one line of §0.1 prose and still did not land it.
//!
//! So: `boyko_render` because it is in the workspace's `default-members`, which is what makes a
//! bare `cargo test` reach it, and because this crate needs no GPU, no `dxc` and no corpus to
//! build a test binary. ⚠️ A standing hazard when checking any of this: `cargo check --all-targets`
//! at the repository root is vacuum-green on the virtual manifest, so "the test is in the
//! workspace" is not evidence that it runs. Look for it in the output.
//!
//! # Line-ending normalisation, which is not a detail
//!
//! The hash is taken over the file with `\r\n` collapsed to `\n`. This repository has
//! `core.autocrlf` behaviour active — git reports "LF will be replaced by CRLF" when it touches
//! these documents — so a hash over raw bytes would be a hash of the checkout configuration rather
//! than of the content, and would red on a coworker's machine while nothing had changed. The
//! normalisation makes the tripwire fire for edits and only for edits.
//!
//! # What is NOT gated here
//!
//! Only this one file. `docs/VG-CAMPAIGN-CLAIM.toml` is deliberately **not** hashed — its fields
//! are required to change exactly once, from `PENDING` to an answer, and hashing a file whose
//! schedule requires it to change is the Rev 2 failure. It is gated instead by the `PENDING`
//! sentinel discipline `goldens/PINS.toml` defines. And the *content* of the thresholds is not
//! checked here at all: whether a rule is sound, dimensionally coherent or able to go red is
//! §14.4's business and an adversarial review's, not a hash's.

use std::path::PathBuf;

/// The sha256 of `docs/VG-CAMPAIGN-THRESHOLDS.toml` with `\r\n` normalised to `\n`, as of the revision the frozen file itself
/// records. ⚠️ This doc named a revision directly and drifted from the file's own
/// `frozen_at_revision` within one revision — an exhaustive disjunction in which both branches
/// are defects. A second copy of a version number is a second thing to go stale.
///
/// **This is the authoring-time baseline, not yet the campaign freeze.** The frozen file's own
/// `freeze_begins_at` says the freeze begins when R0a records this hash into
/// `docs/VG-R0-REFERENCE-RIG.toml`, and R0a has not run — so until then an edit to the thresholds
/// is *authoring*, and updating this literal in the same commit is the legitimate response.
///
/// What the tripwire buys before R0a is exactly what the file's `schema_version` /
/// `frozen_at_revision` fields were supposed to buy and did not: **an edit cannot be silent.**
/// Those two fields were stale through Rev 4, Rev 5 and Rev 7 — three revisions of a staleness
/// marker going stale — because nothing checked them. After R0a the policy changes and this literal
/// moves only by a dated amendment in the plan's §11.1, with the rig file updated in the same act.
const THRESHOLDS_SHA256: &str = "1d51e6501b05f508adb4d293c2e2f9ec9aef7327a1c74949b60355f7aafbe872";

/// Repo-relative path from this crate's manifest directory.
const THRESHOLDS_REL: &str = "../../docs/VG-CAMPAIGN-THRESHOLDS.toml";

fn thresholds_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(THRESHOLDS_REL)
}

/// Reads the frozen file and normalises `\r\n` to `\n` — see the module doc.
fn normalised_bytes() -> Vec<u8> {
    let path = thresholds_path();
    let raw = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\r' && i + 1 < raw.len() && raw[i + 1] == b'\n' {
            i += 1; // skip the CR, keep the LF
            continue;
        }
        out.push(raw[i]);
        i += 1;
    }
    out
}

/// SHA-256 (FIPS 180-4), in-house because this crate carries no third-party dependencies and a
/// tripwire that needs one would not run where it is needed. Gated by a known-answer test below —
/// a wrong implementation would be perfectly *stable*, so the freeze would pass while hashing
/// something other than SHA-256, and nothing would ever say so.
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 64];
    for chunk in msg.chunks_exact(64) {
        for (i, word) in w.iter_mut().enumerate().take(16) {
            let b = &chunk[i * 4..i * 4 + 4];
            *word = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
        let (mut e, mut f, mut g, mut hh) = (h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{word:08x}"));
    }
    out
}

/// Known-answer test for the hash itself. Without this, a wrong implementation is *stable* — the
/// freeze below would pass forever while hashing something that is not SHA-256, and the campaign
/// would record a digest nobody else can reproduce. Vectors from FIPS 180-4.
#[test]
fn sha256_matches_the_published_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "SHA-256 of the empty string"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "SHA-256 of \"abc\""
    );
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        "SHA-256 of the 56-byte multi-block vector — exercises the length-padding boundary this \
         implementation gets wrong first if it is wrong at all"
    );
}

/// The tripwire. `docs/VG-CAMPAIGN-THRESHOLDS.toml` hashes to the recorded value.
#[test]
fn vg_campaign_thresholds_are_unchanged() {
    let actual = sha256_hex(&normalised_bytes());
    assert_eq!(
        actual, THRESHOLDS_SHA256,
        "docs/VG-CAMPAIGN-THRESHOLDS.toml has changed.\n\
         If this is a deliberate authoring edit (R0a has not run, so the campaign freeze has not \
         begun), update THRESHOLDS_SHA256 in this file IN THE SAME COMMIT, and update the frozen \
         file's own `schema_version` and `frozen_at_revision` too — those went stale for three \
         revisions running because nothing checked them.\n\
         After R0a records the hash, this literal moves ONLY by a dated amendment in the plan's \
         §11.1 with docs/VG-R0-REFERENCE-RIG.toml updated in the same act. An unexplained change \
         here is a threshold edit, which is the one thing the two-file split exists to make \
         impossible to do quietly."
    );
}

/// The sensitivity control. A gate that cannot detect a change is vacuously green, and this one
/// guards a file whose entire purpose is to not change — so "it passed" must mean something.
///
/// One byte of the real file's content is flipped **in memory** and must produce a different
/// digest. Deliberately a digit inside a threshold rather than a comment character: the failure
/// this tripwire exists to catch is someone quietly retuning a number, and a control that only
/// proves comments are hashed would prove the wrong thing.
#[test]
fn the_freeze_is_sensitive_to_a_single_changed_threshold_digit() {
    let bytes = normalised_bytes();
    let text = String::from_utf8(bytes.clone()).expect("the frozen file is UTF-8");

    // ⚠️ Anchored to the KEY LINE, not to the first textual match, and that distinction is the
    // whole control. The first version of this test used
    // `text.replacen("d_est_min = 1.0", "d_est_min = 2.0", 1)` — and that string occurs TWICE in
    // the frozen file: once inside a comment discussing where the threshold sits relative to the
    // instrument's ceiling, and once as the live key. `replacen` took the comment. The control
    // passed, proving only that a comment byte is hashed, which this test's own doc calls proving
    // the wrong thing; and the invariant guard below was satisfied by the comment too, so deleting
    // `[k1].d_est_min` outright would have left this green and silent. Found by an adversarial
    // review of the revision that shipped it, and confirmed by one `grep -c`.
    // `split_inclusive` keeps each line's terminator, so reassembly is byte-exact and the only
    // difference between `mutated` and `text` is the one digit. A `lines().join("\n")` would drop
    // the trailing newline and change the digest all by itself — which would make the assertion
    // below pass without the mutation doing anything, the same vacuity one layer down.
    let is_key_line = |l: &str| {
        let t = l.trim_start();
        !t.starts_with('#') && t.starts_with("d_est_min") && t.contains('=')
    };
    let key_line_count = text.split_inclusive('\n').filter(|l| is_key_line(l)).count();
    assert_eq!(
        key_line_count, 1,
        "invariant: exactly one non-comment line must assign `d_est_min` for this control to be \
         meaningful — found {key_line_count}. If K1's threshold was renamed or moved, re-derive \
         this test rather than deleting it."
    );
    let mut mutated = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if is_key_line(line) {
            mutated.push_str(&line.replace("1.0", "2.0"));
        } else {
            mutated.push_str(line);
        }
    }
    assert_ne!(
        mutated, text,
        "invariant: the mutation must actually change the text — otherwise everything below \
         compares a file to itself"
    );
    assert_eq!(
        mutated.len(),
        text.len(),
        "invariant: the mutation is one digit for one digit, so the length must be unchanged — a \
         length change means reassembly altered something other than the threshold"
    );

    assert_ne!(
        sha256_hex(mutated.as_bytes()),
        THRESHOLDS_SHA256,
        "RED: retuning K1's decision threshold produced the SAME digest. The freeze is blind and \
         the assertion above is vacuously green. This is a finding about the tripwire — do not \
         retune the mutation until it passes."
    );
    assert_eq!(
        sha256_hex(&bytes),
        THRESHOLDS_SHA256,
        "the unmutated bytes must still hash to the recorded value — otherwise the control above \
         proves nothing about the live file"
    );
}

/// The highest revision number this file's own comment markers claim edited it.
///
/// Matches `Rev N` / `REV N` case-insensitively. `revision` does not match: the scan requires
/// whitespace and then a digit immediately after `rev`.
fn newest_rev_marker(text: &str) -> Option<u32> {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut best = None;
    let mut i = 0usize;
    while let Some(hit) = lower[i..].find("rev") {
        let mut j = i + hit + 3;
        // Require at least one space, then digits.
        let space_start = j;
        while j < bytes.len() && bytes[j] == b' ' {
            j += 1;
        }
        if j > space_start {
            let digits_start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > digits_start
                && let Ok(n) = lower[digits_start..j].parse::<u32>()
            {
                best = Some(best.map_or(n, |b: u32| b.max(n)));
            }
        }
        i = i + hit + 3;
    }
    best
}

/// The revision named by the `frozen_at_revision` field.
fn frozen_at_revision(text: &str) -> u32 {
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("frozen_at_revision"))
        .expect("invariant: the frozen file must carry frozen_at_revision");
    newest_rev_marker(line).expect("frozen_at_revision must name a `Rev N`")
}

/// Binds the two staleness markers to something, which nothing did.
///
/// ⚠️ `schema_version` and `frozen_at_revision` exist so that an edit to this file cannot be
/// silent. They went stale in Rev 4, Rev 5 and Rev 7 — a staleness marker going stale, three times
/// — and each occurrence was caught by an adversarial reader rather than by a check. The
/// symbol-reachability sweep cannot catch them either: they are in `PROVENANCE_KEYS`, exempted
/// from the one class that would have noticed, and the exemption is correct on its own terms (the
/// plan has no reason to cite them).
///
/// The binding that works without inventing a false red: this file's own edit discipline is that
/// every content edit records itself as a `REV N` comment, so the **newest marker** and
/// `frozen_at_revision` must name the same revision. A revision that does not touch this file moves
/// neither, so it cannot red spuriously; a revision that edits the file must do both, in the same
/// act that moves the digest above. The plan's revision number is deliberately NOT the reference —
/// binding to it would red every time the plan advanced without this file changing, which is the
/// normal case and would train the reader to re-stamp the field without thinking.
#[test]
fn the_newest_edit_marker_and_the_provenance_field_name_the_same_revision() {
    let bytes = normalised_bytes();
    let text = String::from_utf8(bytes).expect("the frozen file is UTF-8");

    let newest = newest_rev_marker(&text).expect("the file must carry at least one `Rev N` marker");
    let frozen = frozen_at_revision(&text);

    assert_eq!(
        frozen, newest,
        "docs/VG-CAMPAIGN-THRESHOLDS.toml: frozen_at_revision names Rev {frozen} while the newest \
         `REV N` comment marker in the file is Rev {newest}.\n\
         If this file was edited, record the edit as a `# REV {newest} -- ...` marker AND bump \
         frozen_at_revision and schema_version, all in the commit that moves THRESHOLDS_SHA256.\n\
         If it was not edited, neither number should have moved."
    );
}
