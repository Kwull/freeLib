//! `phonetic_key` / `name_rank` against the vectors shared with the web app
//! (`docs/web/phonetic-vectors.json`, also checked by `web/tests-unit/phonetic.test.ts`).

use freelib_catalog::normalize;
use freelib_catalog::search::name_rank;
use freelib_catalog::text::phonetic_key;

#[test]
fn shared_vectors() {
    let raw = include_str!("../../../../docs/web/phonetic-vectors.json");
    let doc: serde_json::Value = serde_json::from_str(raw).expect("valid JSON");
    let keys = doc["keys"].as_array().expect("keys");
    assert!(keys.len() > 50);
    for v in keys {
        let input = v["input"].as_str().unwrap();
        assert_eq!(
            phonetic_key(&normalize(input)),
            v["key"].as_str().unwrap(),
            "phonetic_key({input:?})"
        );
    }
    let ranks = doc["ranks"].as_array().expect("ranks");
    for v in ranks {
        let name = normalize(v["name"].as_str().unwrap());
        let tokens: Vec<String> = normalize(v["query"].as_str().unwrap())
            .split(' ')
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect();
        assert_eq!(
            name_rank(&name, &tokens) as u64,
            v["rank"].as_u64().unwrap(),
            "name_rank({name:?}, {tokens:?})"
        );
    }
}
