//! Age suitability estimate of a book ("0+", "6+", "12+", "16+", "18+" or unknown).
//!
//! **A heuristic**, not a rating: it only looks at the FB2 genre codes the library assigned
//! (through the genre ids of `genres.json`) and at a few words in the INPX keywords. Nobody
//! reviewed the books; a children's genre on an adult book (or no genre at all) gives a wrong or
//! missing estimate. The rules, strongest first:
//!
//! 1. Adult genres (`love_erotica`, `love_hard`, `home_sex`) or an adult marker in the keywords
//!    (`18+`, `эротика`, `erotica`, `порно`) → **18+**.
//! 2. An explicit age marker in the keywords (`0+`, `6+`, `12+`, `16+`) → that age.
//! 3. Mature genres (horror, thrillers, serial killers, romance, counterculture, sex psychology)
//!    → **16+**, even when a children's genre is also present.
//! 4. Children's genres → the highest of their levels: tales, nursery verse and children's
//!    folklore **0+**; children's prose, classics, education, folk tales **6+**; children's
//!    adventure / mystery / science fiction, young adult and gamebooks **12+**.
//! 5. Children's words in the keywords (`для детей`, `детская`, `сказк…`, `for children`) → 6+;
//!    teen words (`для подростков`, `young adult`, `подростк…`) → 12+.
//! 6. Otherwise unknown ([`None`]).

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::genres::{canonical_code, genres};

/// The age levels, in order.
pub const KIDS_AGES: [u8; 5] = [0, 6, 12, 16, 18];

/// Stored in compact per-book tables for "unknown".
pub const AGE_UNKNOWN: u8 = u8::MAX;

/// Level of one FB2 genre code: `Some(age)` for children's (0/6/12), mature (16) and adult (18)
/// genres, `None` for neutral ones.
fn code_level(code: &str) -> Option<u8> {
    Some(match code {
        "love_erotica" | "love_hard" | "home_sex" => 18,
        "sf_horror"
        | "det_maniac"
        | "thriller"
        | "thriller_psychology"
        | "thriller_medical"
        | "prose_counter"
        | "psy_sex_and_family"
        | "love"
        | "love_all"
        | "love_contemporary"
        | "love_history"
        | "love_short"
        | "love_sf"
        | "love_detective"
        | "det_hard" => 16,
        "child_tale" | "child_tale_rus" | "child_verse" | "child_folklore" => 0,
        "children" | "child_all" | "child_prose" | "child_education" | "child_classical"
        | "foreign_children" | "folk_tale" | "fairy_fantasy" => 6,
        "child_adv" | "child_det" | "child_sf" | "ya" | "prose_game" => 12,
        _ => return None,
    })
}

/// Genre id → level (built once from the genre table's codes).
fn levels() -> &'static HashMap<u16, u8> {
    static L: OnceLock<HashMap<u16, u8>> = OnceLock::new();
    L.get_or_init(|| {
        let mut m = HashMap::new();
        for g in genres().all() {
            // a genre with several codes takes the strongest signal among them
            let lv = g
                .keys
                .iter()
                .filter_map(|k| code_level(&canonical_code(k)))
                .max();
            if let Some(lv) = lv {
                m.insert(g.id, lv);
            }
        }
        m
    })
}

/// Keyword signals: (adult, explicit marker, children's words, teen words).
fn keyword_signals(keywords: &str) -> (bool, Option<u8>, bool, bool) {
    if keywords.is_empty() {
        return (false, None, false, false);
    }
    let k = keywords.to_lowercase();
    let adult = ["18+", "эроти", "erotic", "порно", "porn"]
        .iter()
        .any(|w| k.contains(w));
    // explicit markers: "0+", "6+", "12+", "16+" as separate tokens
    let mut marker = None;
    for tok in k.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        match tok.trim() {
            "0+" => marker = marker.max(Some(0)),
            "6+" => marker = marker.max(Some(6)),
            "12+" => marker = marker.max(Some(12)),
            "16+" => marker = marker.max(Some(16)),
            _ => {}
        }
    }
    let kids = [
        "для детей",
        "детская",
        "детские",
        "детский",
        "сказк",
        "for children",
    ]
    .iter()
    .any(|w| k.contains(w));
    let teens = ["подростк", "young adult", "для юношества"]
        .iter()
        .any(|w| k.contains(w));
    (adult, marker, kids, teens)
}

/// The age estimate of a book from its genre ids and INPX keywords (see the module docs).
pub fn age_for(genre_ids: &[u16], keywords: &str) -> Option<u8> {
    let lv = levels();
    let mut adult = false;
    let mut mature = false;
    let mut child: Option<u8> = None;
    for g in genre_ids {
        match lv.get(g) {
            Some(18) => adult = true,
            Some(16) => mature = true,
            Some(&a) => child = child.max(Some(a)),
            None => {}
        }
    }
    let (kw_adult, marker, kw_kids, kw_teens) = keyword_signals(keywords);
    if adult || kw_adult {
        return Some(18);
    }
    if let Some(m) = marker {
        return Some(m);
    }
    if mature {
        return Some(16);
    }
    if child.is_some() {
        return child;
    }
    if kw_teens {
        return Some(12);
    }
    if kw_kids {
        return Some(6);
    }
    None
}

/// Compact form for per-book tables: the age, or [`AGE_UNKNOWN`].
pub fn age_code(genre_ids: &[u16], keywords: &str) -> u8 {
    age_for(genre_ids, keywords).unwrap_or(AGE_UNKNOWN)
}

/// Display label: `"0+"`, `"6+"`, …
pub fn age_label(age: u8) -> String {
    format!("{age}+")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(codes: &[&str]) -> Vec<u16> {
        codes
            .iter()
            .map(|c| {
                genres()
                    .by_code(c)
                    .unwrap_or_else(|| panic!("no genre {c}"))
            })
            .collect()
    }

    #[test]
    fn genres_decide() {
        assert_eq!(age_for(&ids(&["child_tale"]), ""), Some(0));
        assert_eq!(age_for(&ids(&["child_prose"]), ""), Some(6));
        assert_eq!(age_for(&ids(&["child_tale", "child_adv"]), ""), Some(12));
        assert_eq!(age_for(&ids(&["ya"]), ""), Some(12));
        assert_eq!(age_for(&ids(&["sf_fantasy"]), ""), None);
        assert_eq!(age_for(&[], ""), None);
        // mature beats children's
        assert_eq!(age_for(&ids(&["child_sf", "sf_horror"]), ""), Some(16));
        assert_eq!(age_for(&ids(&["love_contemporary"]), ""), Some(16));
        // adult beats everything
        assert_eq!(age_for(&ids(&["child_tale", "love_erotica"]), ""), Some(18));
        assert_eq!(age_for(&ids(&["home_sex", "home_health"]), ""), Some(18));
    }

    #[test]
    fn keywords_decide() {
        assert_eq!(age_for(&[], "Сказки, для детей"), Some(6));
        assert_eq!(age_for(&[], "фэнтези, для подростков"), Some(12));
        assert_eq!(age_for(&ids(&["sf"]), "эротика"), Some(18));
        assert_eq!(age_for(&ids(&["child_tale"]), "фэнтези, 16+"), Some(16));
        assert_eq!(age_for(&ids(&["sf"]), "12+, магия"), Some(12));
        assert_eq!(age_for(&ids(&["sf"]), "магия, приключения"), None);
        // "1812+" is not a marker
        assert_eq!(age_for(&[], "война 1812+"), None);
    }

    #[test]
    fn codes() {
        assert_eq!(age_code(&[], ""), AGE_UNKNOWN);
        assert_eq!(age_label(12), "12+");
    }
}
