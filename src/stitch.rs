//! Joins the captured frames, top to bottom, into one long image.
//!
//! Two frames taken before and after scrolling share some rows (the overlap). We find the
//! biggest overlap by comparing a fingerprint (hash) of each row, drop the repeated rows, and
//! append the rest.
//!
//! Each row is cut into 16 vertical slices, and every slice gets its own fingerprint.
//!
//! 1. **Find the offset by voting.** Every slice that has content and is distinctive (it
//!    appears only a few times on the page) of every row of the new frame looks for the same
//!    slice in the old frame. Each hit votes for "the new frame starts N rows down". The true
//!    offset collects the most votes.
//! 2. **Trim.** Rows at the top or bottom of the overlap that do not repeat are cut off. These
//!    are things that stay fixed on screen while the page scrolls: a sticky header, a footer.
//! 3. **Verify** the trimmed overlap slice by slice:
//!    - A slice that matches in (almost) every row is *stable*. A slice that keeps changing is
//!      *volatile* (a minimap, a scrollbar, an animation) and is ignored.
//!    - A slice that also matches when shifted by one row is *static* (a fixed sidebar rule). It
//!      would "prove" any overlap, so it is ignored too.
//!    - Only slices that contain something count as evidence, because plain background
//!      matches anywhere.
//!    - Enough evidence must sit in stable slices, and that must be a fair share of the total.
//! 4. Colours are compared with the lowest 2 bits dropped, to ignore tiny rendering noise.

use std::collections::HashMap;

use crate::capture::Shot;

/// Fewest rows that count as an overlap.
const MIN_OVERLAP: usize = 8;
/// Fewest evidence cells (row x slice with content) in stable slices.
const MIN_EVIDENCE: u32 = 5;
/// Vertical slices per row.
const SEGS: usize = 16;
/// At least this percentage of all evidence must sit in informative slices.
const MIN_SHARE: u32 = 20;
/// A slice that occurs more often than this in the old frame is not distinctive (it is a
/// repeating pattern or a static element) and is not used for voting.
const MAX_DUP: usize = 6;
/// Fewest votes for an offset to be looked at.
const MIN_VOTES: u32 = 6;
/// How many of the best offsets are verified.
const CANDIDATES: usize = 6;
/// After trimming, the repeating part must still be at least this % of the overlap.
const MIN_CORE: usize = 25;

#[cfg(test)]
pub struct Stitched {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    /// How many joins had no detectable overlap (content may be missing there).
    pub gaps: usize,
}

#[derive(Clone, Copy)]
struct RowInfo {
    /// Fingerprint of each slice (flat slices hash their colour).
    seg: [u64; SEGS],
    /// Bit `s` is set if slice `s` contains something (is not one flat colour).
    mask: u16,
}

/// Joins frames one at a time, as they are captured, so a missing overlap can be reported right
/// away and the result is ready the moment the user copies it.
pub struct Stitcher {
    /// Width of every row in pixels (0 = taken from the first frame; later frames are cut or
    /// padded to it).
    w: usize,
    /// Pixels at the right edge left out of the comparison (scrollbar).
    ignore_right: usize,
    rgba: Vec<u8>,
    infos: Vec<RowInfo>,
    /// Rows each frame added, and whether its join found no overlap.
    frames: Vec<(usize, bool)>,
}

impl Stitcher {
    pub fn new(ignore_right: u32) -> Stitcher {
        Stitcher { w: 0, ignore_right: ignore_right as usize, rgba: Vec::new(), infos: Vec::new(), frames: Vec::new() }
    }

    /// Adds a frame under the others. Returns `false` if it had no detectable overlap with the
    /// previous one (it is then joined edge to edge, R21).
    pub fn push(&mut self, shot: &Shot) -> bool {
        if self.w == 0 {
            self.w = shot.w as usize;
        }
        let w = self.w;
        let cmp_w = if w > 4 * self.ignore_right { w - self.ignore_right } else { w };
        let take = w.min(shot.w as usize) * 4;
        let rows: Vec<std::borrow::Cow<[u8]>> = (0..shot.h as usize)
            .map(|y| {
                let src = &shot.rgba[y * shot.w as usize * 4..][..take];
                if take == w * 4 {
                    std::borrow::Cow::Borrowed(src)
                } else {
                    // A narrower frame is padded with see-through pixels.
                    let mut row = src.to_vec();
                    row.resize(w * 4, 0);
                    std::borrow::Cow::Owned(row)
                }
            })
            .collect();
        let new_infos: Vec<RowInfo> = rows.iter().map(|r| row_info(r, cmp_w)).collect();

        let (skip, gap) = if self.frames.is_empty() {
            (0, false)
        } else {
            match find_overlap(&self.infos, &new_infos) {
                Some(k) => (k, false),
                None => (0, true),
            }
        };
        self.rgba.reserve((rows.len() - skip) * w * 4);
        self.infos.reserve(rows.len() - skip);
        for (row, info) in rows.iter().zip(&new_infos).skip(skip) {
            self.rgba.extend_from_slice(row);
            self.infos.push(*info);
        }
        self.frames.push((rows.len() - skip, gap));
        !gap
    }

    /// Removes the last frame added.
    pub fn pop(&mut self) {
        if let Some((rows, _)) = self.frames.pop() {
            let keep = self.infos.len() - rows;
            self.infos.truncate(keep);
            self.rgba.truncate(keep * self.w * 4);
        }
        if self.frames.is_empty() {
            // Back to no frames: the width may change again (the frame unlocks).
            self.w = 0;
        }
    }

    pub fn frames(&self) -> usize {
        self.frames.len()
    }

    /// How many joins had no detectable overlap.
    pub fn gaps(&self) -> usize {
        self.frames.iter().filter(|(_, gap)| *gap).count()
    }

    /// Size of the joined image so far, if there is one.
    pub fn size(&self) -> Option<(u32, u32)> {
        (!self.infos.is_empty()).then_some((self.w as u32, self.infos.len() as u32))
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Hands over the joined pixels (used when saving, at the end of a session).
    pub fn take_rgba(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.rgba)
    }

    /// Gives back pixels handed over by `take_rgba` (the save failed, the session goes on).
    pub fn restore_rgba(&mut self, rgba: Vec<u8>) {
        if rgba.len() == self.infos.len() * self.w * 4 {
            self.rgba = rgba;
        }
    }
}

/// Joins all `shots` at once (tests; the app joins frame by frame with `Stitcher`).
/// `ignore_right`: pixels at the right edge left out of the
/// comparison (scrollbar). Frames are cut to the narrowest one.
#[cfg(test)]
pub fn stitch(shots: &[Shot], ignore_right: u32) -> Option<Stitched> {
    let w = shots.iter().map(|s| s.w).min()? as usize;
    let mut st = Stitcher::new(ignore_right);
    st.w = w;
    for shot in shots {
        st.push(shot);
    }
    let (w, h) = st.size()?;
    let gaps = st.gaps();
    Some(Stitched { w, h, rgba: st.take_rgba(), gaps })
}

/// Hashes each slice of a row and notes which slices have content, in one pass over the pixels.
/// Pixels are read as whole `u32`s with the lowest 2 bits of every channel masked off.
fn row_info(row: &[u8], cmp_w: usize) -> RowInfo {
    const NOISE: u32 = u32::from_ne_bytes([0xFC; 4]);
    let (px, _) = row[..cmp_w * 4].as_chunks::<4>();
    let mut info = RowInfo { seg: [0; SEGS], mask: 0 };
    for s in 0..SEGS {
        let seg = &px[s * cmp_w / SEGS..(s + 1) * cmp_w / SEGS];
        let first = seg.first().map_or(0, |p| u32::from_ne_bytes(*p) & NOISE);
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a, one pixel per step
        let mut flat = true;
        for p in seg {
            let v = u32::from_ne_bytes(*p) & NOISE;
            hash = (hash ^ v as u64).wrapping_mul(0x0000_0100_0000_01b3);
            flat &= v == first;
        }
        info.seg[s] = hash;
        if !flat {
            info.mask |= 1 << s;
        }
    }
    info
}

fn slices(mut mask: u16) -> impl Iterator<Item = usize> {
    std::iter::from_fn(move || {
        if mask == 0 {
            return None;
        }
        let s = mask.trailing_zeros() as usize;
        mask &= mask - 1;
        Some(s)
    })
}

/// Number of rows at the end of `a` that repeat at the start of `b`.
fn find_overlap(a: &[RowInfo], b: &[RowInfo]) -> Option<usize> {
    let w = a.len().min(b.len());
    if w < MIN_OVERLAP {
        return None;
    }
    // Only the last `w` rows of `a` can overlap with `b`.
    let a_start = a.len() - w;

    // Where does each slice (with content) occur in the old frame?
    let mut index: HashMap<(usize, u64), Vec<u32>> = HashMap::new();
    for (j, row) in a[a_start..].iter().enumerate() {
        for s in slices(row.mask) {
            index.entry((s, row.seg[s])).or_default().push(j as u32);
        }
    }
    let distinctive = |s: usize, hash: u64| index.get(&(s, hash)).is_some_and(|l| l.len() <= MAX_DUP);

    // Vote: row `i` of `b` equals row `j` of the window, so `b` starts at window row `j - i`.
    // Offsets run from `-(b.len() - 1)` to `w - 1`; slot `j - i + b.len()` counts each one.
    let mut votes = vec![0u32; w + b.len()];
    for (i, row) in b.iter().enumerate() {
        for s in slices(row.mask) {
            if let Some(list) = index.get(&(s, row.seg[s]))
                && list.len() <= MAX_DUP
            {
                for &j in list {
                    votes[j as usize + b.len() - i] += 1;
                }
            }
        }
    }
    let mut best: Vec<(i64, u32)> = votes
        .iter()
        .enumerate()
        .filter(|&(_, &v)| v >= MIN_VOTES)
        .map(|(slot, &v)| (slot as i64 - b.len() as i64, v))
        .collect();
    best.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    best.truncate(CANDIDATES);

    // Verify the candidates and keep the one with the longest repeating part.
    let mut winner: Option<(usize, usize)> = None; // (overlap rows, repeating rows)
    for (offset, _) in best {
        let start = a_start as i64 + offset; // row of `a` where `b` begins
        if start < 0 {
            continue;
        }
        let k = a.len() - start as usize;
        if k < MIN_OVERLAP || k > b.len() {
            continue;
        }
        if let Some(core) = verify(&a[start as usize..], &b[..k], &distinctive)
            && winner.is_none_or(|(_, best_core)| core > best_core)
        {
            winner = Some((k, core));
        }
    }
    winner.map(|(k, _)| k)
}

/// Checks that `b` (same length as `a`) repeats `a` once the non-repeating bands at its top and
/// bottom are trimmed. Returns the number of rows that remain.
fn verify(a: &[RowInfo], b: &[RowInfo], distinctive: &dyn Fn(usize, u64) -> bool) -> Option<usize> {
    let k = a.len();
    // A row supports the offset if a distinctive slice is identical in both frames.
    let supports = |i: usize| {
        slices(a[i].mask | b[i].mask).any(|s| a[i].seg[s] == b[i].seg[s] && distinctive(s, b[i].seg[s]))
    };
    let lo = (0..k).find(|&i| supports(i))?;
    let hi = (0..k).rev().find(|&i| supports(i))? + 1;
    let core = hi - lo;
    if core < MIN_OVERLAP || core * 100 < k * MIN_CORE {
        return None;
    }

    let (a, b) = (&a[lo..hi], &b[lo..hi]);
    let (ev, ok) = tally(a, b, 0..core, 0);
    let (ev_shift, ok_shift) = tally(a, b, 0..core - 1, 1);
    let (stable, total) = verdict(&ev, &ok, &ev_shift, &ok_shift);
    (stable >= MIN_EVIDENCE && stable * 100 >= total * MIN_SHARE).then_some(core)
}

/// For each slice: how many of the given rows have content there, and how many of those repeat.
/// `shift` compares row `i` of `a` with row `i + shift` of `b`.
fn tally(
    a: &[RowInfo],
    b: &[RowInfo],
    rows: impl Iterator<Item = usize>,
    shift: usize,
) -> ([u32; SEGS], [u32; SEGS]) {
    let mut evidence = [0u32; SEGS];
    let mut matched = [0u32; SEGS];
    for i in rows {
        let (ra, rb) = (&a[i], &b[i + shift]);
        let mut live = ra.mask | rb.mask;
        while live != 0 {
            let s = live.trailing_zeros() as usize;
            live &= live - 1;
            evidence[s] += 1;
            matched[s] += (ra.seg[s] == rb.seg[s]) as u32;
        }
    }
    (evidence, matched)
}

fn mostly(matched: u32, evidence: u32) -> bool {
    evidence > 0 && matched * 100 >= evidence * 90
}

/// `(evidence in informative slices, evidence in all slices)`.
/// A slice is informative if it repeats at the candidate overlap (>= 90% of its evidence) but
/// does NOT also repeat when `b` is shifted by one row. A slice that matches at every shift is
/// static (a fixed sidebar, a vertical rule) and would "prove" any overlap. Slices that keep
/// changing (minimap, scrollbar, animation) simply fail to repeat and are ignored.
fn verdict(
    evidence: &[u32; SEGS],
    matched: &[u32; SEGS],
    ev_shift: &[u32; SEGS],
    ok_shift: &[u32; SEGS],
) -> (u32, u32) {
    let total: u32 = evidence.iter().sum();
    let stable: u32 = (0..SEGS)
        .filter(|&s| mostly(matched[s], evidence[s]) && !mostly(ok_shift[s], ev_shift[s]))
        .map(|s| evidence[s])
        .sum();
    (stable, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic noise, so tests don't need a random-number crate.
    fn rng(seed: &mut u64) -> u8 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 24) as u8
    }

    /// A tall "page": every row is different noise, except `blank` rows which are plain white.
    fn page(w: usize, h: usize, blank: &[std::ops::Range<usize>]) -> Vec<u8> {
        let mut seed = 0x1234_5678_9abc_def1;
        let mut out = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let is_blank = blank.iter().any(|r| r.contains(&y));
            for _ in 0..w {
                if is_blank {
                    out.extend_from_slice(&[255, 255, 255, 255]);
                } else {
                    out.extend_from_slice(&[rng(&mut seed), rng(&mut seed), rng(&mut seed), 255]);
                }
            }
        }
        out
    }

    /// Rows `from..to` of a page as a frame.
    fn frame(page: &[u8], w: usize, from: usize, to: usize) -> Shot {
        Shot { w: w as u32, h: (to - from) as u32, rgba: page[from * w * 4..to * w * 4].to_vec() }
    }

    #[test]
    fn joins_overlapping_frames_exactly() {
        let (w, h) = (40, 1000);
        let p = page(w, h, &[]);
        // Three frames of 400 rows, scrolled by 300 each time (100 rows overlap).
        let shots = [frame(&p, w, 0, 400), frame(&p, w, 300, 700), frame(&p, w, 600, 1000)];
        let s = stitch(&shots, 0).unwrap();
        assert_eq!((s.w as usize, s.h as usize, s.gaps), (w, 1000, 0));
        assert_eq!(s.rgba, p);
    }

    #[test]
    fn single_frame_is_returned_unchanged() {
        let p = page(30, 200, &[]);
        let s = stitch(&[frame(&p, 30, 0, 200)], 0).unwrap();
        assert_eq!((s.h, s.gaps), (200, 0));
        assert_eq!(s.rgba, p);
    }

    #[test]
    fn no_overlap_is_joined_and_flagged() {
        let (w, h) = (40, 600);
        let p = page(w, h, &[]);
        let shots = [frame(&p, w, 0, 300), frame(&p, w, 300, 600)];
        let s = stitch(&shots, 0).unwrap();
        assert_eq!((s.h, s.gaps), (600, 1));
        assert_eq!(s.rgba, p);
    }

    #[test]
    fn identical_frames_do_not_repeat() {
        let p = page(40, 300, &[]);
        let shots = [frame(&p, 40, 0, 300), frame(&p, 40, 0, 300)];
        let s = stitch(&shots, 0).unwrap();
        assert_eq!((s.h, s.gaps), (300, 0));
    }

    #[test]
    fn blank_rows_do_not_cause_false_matches() {
        // Both frames end/start in a big white area. The real overlap (rows 250..400) has content.
        let (w, h) = (40, 900);
        let p = page(w, h, &[100..250, 650..900]);
        let shots = [frame(&p, w, 0, 400), frame(&p, w, 250, 650), frame(&p, w, 500, 900)];
        let s = stitch(&shots, 0).unwrap();
        assert_eq!(s.rgba, p);
        assert_eq!(s.gaps, 0);
    }

    #[test]
    fn frames_of_different_heights() {
        // The user changed the height between frames.
        let (w, h) = (40, 1000);
        let p = page(w, h, &[]);
        let shots = [frame(&p, w, 0, 500), frame(&p, w, 400, 650), frame(&p, w, 600, 1000)];
        let s = stitch(&shots, 0).unwrap();
        assert_eq!((s.h as usize, s.gaps), (1000, 0));
        assert_eq!(s.rgba, p);
    }

    #[test]
    fn scrollbar_column_is_ignored() {
        let (w, h) = (60, 800);
        let mut p = page(w, h, &[]);
        let mut a = frame(&p, w, 0, 400);
        let mut b = frame(&p, w, 300, 700);
        // Paint the right 8 pixels differently in each frame, as a scrollbar thumb would.
        for (shot, val) in [(&mut a, 10u8), (&mut b, 200u8)] {
            for y in 0..shot.h as usize {
                for x in w - 8..w {
                    shot.rgba[(y * w + x) * 4] = val;
                }
            }
        }
        // Make the expected image match what was captured (frame a's scrollbar for rows 0..400).
        for y in 0..400 {
            for x in w - 8..w {
                p[(y * w + x) * 4] = 10;
            }
        }
        let s = stitch(&[a, b], 8).unwrap();
        assert_eq!((s.h, s.gaps), (700, 0));
    }

    #[test]
    fn a_few_noisy_rows_are_tolerated() {
        let (w, h) = (40, 700);
        let p = page(w, h, &[]);
        let a = frame(&p, w, 0, 400);
        let mut b = frame(&p, w, 300, 700);
        // A blinking cursor changes 3 rows inside the overlap.
        for y in 10..13 {
            for x in 0..w {
                b.rgba[(y * w + x) * 4 + 1] ^= 0xF0;
            }
        }
        let s = stitch(&[a, b], 0).unwrap();
        assert_eq!((s.h, s.gaps), (700, 0));
    }

    /// Requirement N5: 10 frames in under 1 second. Only meaningful in a release build.
    #[test]
    #[cfg(not(debug_assertions))]
    fn ten_retina_frames_join_in_under_a_second() {
        let (w, h, frame_h, step) = (2400usize, 8000usize, 1600usize, 700usize);
        let p = page(w, h, &[]);
        let shots: Vec<Shot> = (0..10).map(|i| frame(&p, w, i * step, i * step + frame_h)).collect();
        let t = std::time::Instant::now();
        let s = stitch(&shots, 48).unwrap();
        let took = t.elapsed();
        eprintln!("10 frames of {w}x{frame_h}: {took:?}");
        assert_eq!((s.h as usize, s.gaps), (9 * step + frame_h, 0));
        assert!(took.as_millis() < 1000, "took {took:?}");
    }

    /// Text-editor-like page: short lines of noise on the left, blank elsewhere, blank lines
    /// between them. `volatile` returns the right-hand strip (a minimap) for each frame.
    fn editor_page(w: usize, h: usize) -> Vec<u8> {
        let mut seed = 0x0bad_cafe_dead_beef;
        let mut out = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let text = y % 3 != 2; // every third row is blank
            let len = 8 + (rng(&mut seed) as usize % 24); // short lines
            for x in 0..w {
                if text && x < len {
                    out.extend_from_slice(&[rng(&mut seed), rng(&mut seed), rng(&mut seed), 255]);
                } else {
                    out.extend_from_slice(&[30, 30, 34, 255]);
                }
            }
        }
        out
    }

    #[test]
    fn a_changing_minimap_column_does_not_break_the_join() {
        // Like VS Code: the right ~15% of every frame is a minimap that changes with scrolling.
        let (w, h) = (200, 900);
        let p = editor_page(w, h);
        let mut a = frame(&p, w, 0, 400);
        let mut b = frame(&p, w, 300, 700);
        let mut seed = 99u64;
        for shot in [&mut a, &mut b] {
            for y in 0..shot.h as usize {
                for x in (w * 85 / 100)..w {
                    for c in 0..3 {
                        shot.rgba[(y * w + x) * 4 + c] = rng(&mut seed);
                    }
                }
            }
        }
        let s = stitch(&[a, b], 0).unwrap();
        assert_eq!((s.h, s.gaps), (700, 0));
    }

    #[test]
    fn short_lines_do_not_match_at_the_wrong_offset() {
        let (w, h) = (200, 1200);
        let p = editor_page(w, h);
        let shots = [frame(&p, w, 0, 500), frame(&p, w, 350, 850), frame(&p, w, 700, 1200)];
        let s = stitch(&shots, 0).unwrap();
        assert_eq!((s.h as usize, s.gaps), (1200, 0));
        assert_eq!(s.rgba, p);
    }

    #[test]
    fn unrelated_frames_are_not_joined_by_accident() {
        // Two different pages: must be reported as a gap, never as a fake overlap.
        let (w, h) = (200, 400);
        let a = editor_page(w, h);
        let b: Vec<u8> = {
            let mut seed = 777u64;
            (0..w * h * 4).map(|_| rng(&mut seed)).collect()
        };
        let s = stitch(&[frame(&a, w, 0, h), frame(&b, w, 0, h)], 0).unwrap();
        assert_eq!((s.h as usize, s.gaps), (2 * h, 1));
    }

    #[test]
    fn a_static_stripe_does_not_fake_an_overlap() {
        // A fixed vertical pattern (sidebar rule) is in every row of both frames. The rest of the
        // two frames is unrelated. It must be reported as a gap, not joined at a made-up offset.
        let (w, h) = (200, 400);
        let make = |seed0: u64| -> Shot {
            let mut seed = seed0;
            let mut rgba: Vec<u8> = (0..w * h * 4).map(|_| rng(&mut seed)).collect();
            for y in 0..h {
                for x in 37..50 {
                    let c: [u8; 4] = if x % 2 == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
                    rgba[(y * w + x) * 4..][..4].copy_from_slice(&c);
                }
            }
            Shot { w: w as u32, h: h as u32, rgba }
        };
        let s = stitch(&[make(11), make(22)], 0).unwrap();
        assert_eq!((s.h as usize, s.gaps), (2 * h, 1));
    }

    /// Paints rows `from..to` of a frame with a fixed band (same pixels in every frame), like a
    /// sticky header or footer.
    fn paint_band(shot: &mut Shot, from: usize, to: usize, seed0: u64) {
        let w = shot.w as usize;
        let mut seed = seed0;
        for y in from..to {
            for x in 0..w {
                for c in 0..3 {
                    shot.rgba[(y * w + x) * 4 + c] = rng(&mut seed);
                }
            }
        }
    }

    #[test]
    fn a_sticky_header_does_not_break_the_join() {
        // Like VS Code's sticky scroll: the first 60 rows of every frame stay the same while
        // the page scrolls under them.
        let (w, h) = (200, 1000);
        let p = page(w, h, &[]);
        let mut a = frame(&p, w, 0, 500);
        let mut b = frame(&p, w, 300, 800);
        let mut c = frame(&p, w, 600, 1000);
        for shot in [&mut a, &mut b, &mut c] {
            paint_band(shot, 0, 60, 4242);
        }
        let s = stitch(&[a, b, c], 0).unwrap();
        assert_eq!((s.h as usize, s.gaps), (1000, 0));
    }

    #[test]
    fn a_sticky_header_and_footer_do_not_break_the_join() {
        let (w, h) = (200, 1200);
        let p = page(w, h, &[]);
        let mut a = frame(&p, w, 0, 600);
        let mut b = frame(&p, w, 300, 900);
        for shot in [&mut a, &mut b] {
            paint_band(shot, 0, 60, 4242);
            paint_band(shot, 570, 600, 9999);
        }
        let s = stitch(&[a, b], 0).unwrap();
        assert_eq!((s.h as usize, s.gaps), (900, 0));
    }

    /// Frames saved with SCOTSHREEN_DEBUG_DIR. Run with:
    /// SCOTSHREEN_TEST_FRAMES=/folder cargo test --release real_frames -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_frames() {
        let dir = std::env::var("SCOTSHREEN_TEST_FRAMES").expect("set SCOTSHREEN_TEST_FRAMES");
        let mut shots = Vec::new();
        for n in 1.. {
            let Ok(img) = xcap::image::open(format!("{dir}/frame-{n}.png")) else { break };
            let img = img.to_rgba8();
            shots.push(Shot { w: img.width(), h: img.height(), rgba: img.into_raw() });
        }
        assert!(shots.len() >= 2, "need at least two frames");
        let s = stitch(&shots, 48).unwrap();
        eprintln!("{} frames of {}x{} -> {}x{}, {} gap(s)", shots.len(), shots[0].w, shots[0].h, s.w, s.h, s.gaps);
        let img = xcap::image::RgbaImage::from_raw(s.w, s.h, s.rgba).unwrap();
        img.save(format!("{dir}/stitched.png")).unwrap();
        assert_eq!(s.gaps, 0);
    }

    #[test]
    fn frames_join_one_at_a_time_and_undo_removes_the_last() {
        let (w, h) = (300usize, 900usize);
        let p = page(w, h, &[]);
        let mut st = Stitcher::new(0);
        assert!(st.push(&frame(&p, w, 0, 400)));
        assert!(st.push(&frame(&p, w, 250, 650)));
        assert_eq!(st.size(), Some((w as u32, 650)));
        // Far too far down: no overlap, reported at once.
        assert!(!st.push(&frame(&p, w, 800, 900)));
        assert_eq!((st.frames(), st.gaps()), (3, 1));
        st.pop();
        assert_eq!((st.frames(), st.gaps(), st.size()), (2, 0, Some((w as u32, 650))));
        assert_eq!(st.rgba().len(), w * 650 * 4);
        // Undo back to nothing, then a different width is accepted.
        st.pop();
        st.pop();
        assert_eq!(st.size(), None);
        let q = page(200, 300, &[]);
        st.push(&frame(&q, 200, 0, 100));
        assert_eq!(st.size(), Some((200, 100)));
    }

    #[test]
    fn empty_input_gives_nothing() {
        assert!(stitch(&[], 0).is_none());
    }
}
