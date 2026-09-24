//! Global, static genre table (`data/genres.json`, exported from the Qt app).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Id of the top-level "Прочее" group, the last-resort target for unknown codes.
pub const GENRE_OTHER: u16 = 11;

/// One genre (a top-level group has `parent == 0`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenreDef {
    pub id: u16,
    pub name: String,
    pub parent: u16,
    /// FB2 genre codes mapped to this genre.
    pub keys: Vec<String>,
    /// Display order inside the parent (Qt `sort_index`), `None` for groups.
    pub sort: Option<i32>,
}

/// The loaded genre table with lookup maps.
#[derive(Debug)]
pub struct Genres {
    list: Vec<GenreDef>,
    by_id: HashMap<u16, usize>,
    by_code: HashMap<String, u16>,
    /// code prefix (text before the first `_`) → most likely top-level group
    group_by_prefix: HashMap<String, u16>,
    /// top-level group → its "…: прочее" leaf
    other_of_group: HashMap<u16, u16>,
}

static GENRES: OnceLock<Genres> = OnceLock::new();

/// The embedded genre table (parsed once).
pub fn genres() -> &'static Genres {
    GENRES.get_or_init(|| {
        let list: Vec<GenreDef> =
            serde_json::from_str(include_str!("../data/genres.json")).expect("embedded genres.json is valid");
        Genres::new(list)
    })
}

/// Canonical form of an FB2 genre code as the Qt importer does it: lower-case, spaces → `_`.
pub fn canonical_code(code: &str) -> String {
    code.trim().to_lowercase().replace(' ', "_")
}

fn code_prefix(code: &str) -> &str {
    code.split('_').next().unwrap_or(code)
}

impl Genres {
    fn new(list: Vec<GenreDef>) -> Self {
        let by_id: HashMap<u16, usize> = list.iter().enumerate().map(|(i, g)| (g.id, i)).collect();
        let mut by_code = HashMap::new();
        for g in &list {
            for k in &g.keys {
                by_code.insert(canonical_code(k), g.id);
            }
        }
        // Majority vote of the top-level group per code prefix; ties → lowest group id.
        let mut votes: HashMap<String, HashMap<u16, u32>> = HashMap::new();
        for g in &list {
            let top = top_of(&list, &by_id, g.id);
            for k in &g.keys {
                *votes.entry(code_prefix(&canonical_code(k)).to_string()).or_default().entry(top).or_default() += 1;
            }
        }
        let group_by_prefix = votes
            .into_iter()
            .map(|(p, v)| {
                let best = v.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|x| x.0).unwrap();
                (p, best)
            })
            .collect();
        let mut other_of_group = HashMap::new();
        for g in &list {
            if g.parent != 0 && g.name.to_lowercase().ends_with("прочее") {
                other_of_group.insert(g.parent, g.id);
            }
        }
        Genres { list, by_id, by_code, group_by_prefix, other_of_group }
    }

    /// All genres in file order (groups first, then leaves).
    pub fn all(&self) -> &[GenreDef] {
        &self.list
    }

    pub fn get(&self, id: u16) -> Option<&GenreDef> {
        self.by_id.get(&id).map(|&i| &self.list[i])
    }

    /// Exact lookup of an FB2 code (case-insensitive, spaces = underscores).
    pub fn by_code(&self, code: &str) -> Option<u16> {
        self.by_code.get(&canonical_code(code)).copied()
    }

    /// Map any FB2 code to a genre id. Unknown codes go to the "…: прочее" leaf of the
    /// group their prefix belongs to (`sf_new` → "Фантастика: прочее"), else to [`GENRE_OTHER`].
    /// Returns `None` for an empty code.
    pub fn resolve(&self, code: &str) -> Option<u16> {
        let c = canonical_code(code);
        if c.is_empty() {
            return None;
        }
        if let Some(&id) = self.by_code.get(&c) {
            return Some(id);
        }
        let group = self.group_by_prefix.get(code_prefix(&c)).copied();
        Some(group.and_then(|g| self.other_of_group.get(&g).copied()).unwrap_or(GENRE_OTHER))
    }

    /// Top-level groups (parent == 0), ordered by id.
    pub fn top_level(&self) -> impl Iterator<Item = &GenreDef> {
        self.list.iter().filter(|g| g.parent == 0)
    }

    /// Direct children of `id`, in display order.
    pub fn children(&self, id: u16) -> Vec<&GenreDef> {
        let mut v: Vec<&GenreDef> = self.list.iter().filter(|g| g.parent == id && g.id != id).collect();
        v.sort_by_key(|g| (g.sort.unwrap_or(i32::MAX), g.id));
        v
    }

    /// `id` itself plus all descendants (the tree is two levels deep, but this does not assume it).
    pub fn with_descendants(&self, id: u16) -> Vec<u16> {
        let mut out = vec![id];
        let mut i = 0;
        while i < out.len() {
            let cur = out[i];
            out.extend(self.list.iter().filter(|g| g.parent == cur && g.id != cur).map(|g| g.id));
            i += 1;
        }
        out
    }

    /// Top-level ancestor of `id` (itself when it is a group).
    pub fn top(&self, id: u16) -> u16 {
        top_of(&self.list, &self.by_id, id)
    }
}

fn top_of(list: &[GenreDef], by_id: &HashMap<u16, usize>, mut id: u16) -> u16 {
    for _ in 0..8 {
        match by_id.get(&id) {
            Some(&i) if list[i].parent != 0 => id = list[i].parent,
            _ => break,
        }
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads() {
        let g = genres();
        assert_eq!(g.all().len(), 322);
        assert_eq!(g.get(11).unwrap().name, "Прочее");
        assert!(g.top_level().count() > 20);
    }

    #[test]
    fn codes() {
        let g = genres();
        let sf = g.by_code("sf_heroic").unwrap();
        assert_eq!(g.get(sf).unwrap().parent, 1);
        assert_eq!(g.by_code("SF Heroic"), Some(sf));
        assert_eq!(g.resolve("sf_heroic"), Some(sf));
        // unknown code with a known prefix → group "прочее"
        assert_eq!(g.resolve("sf_brand_new"), Some(118));
        assert_eq!(g.resolve("det_whatever"), Some(400));
        // completely unknown
        assert_eq!(g.resolve("zzz_qqq"), Some(GENRE_OTHER));
        assert_eq!(g.resolve("  "), None);
    }

    #[test]
    fn tree() {
        let g = genres();
        let kids = g.children(1);
        assert!(kids.len() > 10);
        assert!(kids.iter().all(|k| k.parent == 1));
        let d = g.with_descendants(1);
        assert_eq!(d.len(), kids.len() + 1);
        assert_eq!(g.top(118), 1);
        assert_eq!(g.top(1), 1);
    }
}
