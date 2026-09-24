//! `normalize` / `letter_of` against the vectors shared with the web app
//! (`docs/web/normalize-vectors.json`, also checked by `web/tests-unit/normalize.test.ts`).

use freelib_catalog::{letter_of, normalize};

#[test]
fn shared_vectors() {
    let raw = include_str!("../../../../docs/web/normalize-vectors.json");
    let doc: serde_json::Value = serde_json::from_str(raw).expect("valid JSON");
    let vectors = doc["vectors"].as_array().expect("vectors");
    assert!(vectors.len() > 20);
    for v in vectors {
        let input = v["input"].as_str().unwrap();
        let n = normalize(input);
        assert_eq!(n, v["normalized"].as_str().unwrap(), "normalize({input:?})");
        assert_eq!(
            letter_of(&n),
            v["letter"].as_str().unwrap(),
            "letter_of({n:?})"
        );
    }
}
