//! The 22 fixtures that were run through every model's stock
//! `configmerge`; the expected text is the stock binary's standard output.

use dphone_config::merge::{NetContext, merge_layers};
use dphone_config::xml::Element;

#[test]
fn stock_fixtures() {
    let text = include_str!("data/configmerge-fixtures.json");
    let cases: serde_json::Value = serde_json::from_str(text).unwrap();
    let cases = cases.as_array().unwrap();
    assert_eq!(cases.len(), 22);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let inputs = c["inputs"].as_array().unwrap();
        let layers = inputs.iter().map(|i| Element::parse(i.as_str().unwrap()).unwrap());
        let out = merge_layers(layers, &NetContext::default()).to_document();
        assert_eq!(out, c["expected"].as_str().unwrap(), "fixture {name}");
    }
}
