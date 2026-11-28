//! Slide deck selection and loading from Showcase\showcase.json.

use std::fs;
use std::path::Path;

use crate::json_mini;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlideItem {
    pub id: String,
    pub title: String,
    pub quote: String,
    pub author: String,
    pub slide_file: String,
    pub kind: String,
    pub pinned: bool,
}

/// Parse showcase JSON array into SlideItem records.
pub fn parse_slides(json_text: &str) -> Result<Vec<SlideItem>, String> {
    let root = json_mini::parse(json_text)?;
    let arr = root.as_arr().ok_or("showcase.json root is not an array")?;
    let mut items = Vec::with_capacity(arr.len());
    for obj in arr {
        items.push(SlideItem {
            id: obj.get_str("id").unwrap_or("").to_string(),
            title: obj.get_str("title").unwrap_or("").to_string(),
            quote: obj.get_str("quote").unwrap_or("").to_string(),
            author: obj.get_str("author").unwrap_or("").to_string(),
            slide_file: obj.get_str("slide_file").unwrap_or("").to_string(),
            kind: obj.get_str("kind").unwrap_or("classic").to_string(),
            pinned: obj.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false),
        });
    }
    Ok(items)
}

/// Build the full playback deck:
/// - Always starts on one of the classic games (shuffled using `seed`).
/// - Shuffles all remaining games (including any `pinned` game like Path of Exile 2).
pub fn select_deck(all: &[SlideItem], seed: u64) -> Vec<SlideItem> {
    if all.is_empty() {
        return Vec::new();
    }

    let mut classics: Vec<SlideItem> = all
        .iter()
        .filter(|s| !s.pinned && s.kind.eq_ignore_ascii_case("classic"))
        .cloned()
        .collect();
    let other: Vec<SlideItem> = all
        .iter()
        .filter(|s| s.pinned || !s.kind.eq_ignore_ascii_case("classic"))
        .cloned()
        .collect();

    // Deterministic Xorshift shuffle seeded cleanly per launch.
    let mut rng = if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed };
    for i in (1..classics.len()).rev() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let j = (rng as usize) % (i + 1);
        classics.swap(i, j);
    }

    let mut deck = Vec::with_capacity(all.len());
    let mut tail: Vec<SlideItem> = Vec::with_capacity(all.len());
    if !classics.is_empty() {
        deck.push(classics.remove(0));
        tail.extend(classics);
        tail.extend(other);
    } else {
        tail.extend(other);
    }

    for i in (1..tail.len()).rev() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let j = (rng as usize) % (i + 1);
        tail.swap(i, j);
    }

    deck.extend(tail);
    deck
}

/// Load slides from `Showcase\showcase.json` relative to `pkg_root`.
pub fn load_from_package(pkg_root: &Path, seed: u64) -> Vec<SlideItem> {
    let json_path = pkg_root.join("Showcase").join("showcase.json");
    if let Ok(text) = fs::read_to_string(&json_path) {
        if let Ok(items) = parse_slides(&text) {
            return select_deck(&items, seed);
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_items() -> Vec<SlideItem> {
        let json = r#"[
          {"id":"homm3","title":"HoMM3","quote":"q1","author":"","slide_file":"slide_homm3.jpg","kind":"classic"},
          {"id":"fallout2","title":"Fallout 2","quote":"q2","author":"","slide_file":"slide_fallout2.jpg","kind":"classic"},
          {"id":"planescape","title":"Planescape","quote":"q3","author":"","slide_file":"slide_planescape.jpg","kind":"classic"},
          {"id":"poe2","title":"Path of Exile 2","quote":"q4","author":"Hooded","slide_file":"slide_poe2.jpg","kind":"poe","pinned":true}
        ]"#;
        parse_slides(json).unwrap()
    }

    #[test]
    fn deck_starts_with_classic_and_includes_all_items() {
        let items = sample_items();
        for seed in 1..=20 {
            let deck = select_deck(&items, seed);
            assert_eq!(deck.len(), items.len());
            assert_eq!(deck[0].kind, "classic");
            assert!(deck[1..].iter().any(|s| s.id == "poe2"));
        }
    }
}
