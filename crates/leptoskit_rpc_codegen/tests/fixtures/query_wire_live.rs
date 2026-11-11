#[cfg(test)]
mod wire_presence_acceptance {
    use super::*;
    #[test]
    fn live_keeps_null_and_omits_absent_optional_fields() {
        #[derive(Serialize)]
        struct Args {
            note: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            missing: Option<String>,
            items: Vec<String>,
        }
        let value = arguments(&Args {note: None, missing: None, items: vec![]}).unwrap();
        assert_eq!(value, json!({"note": null, "items": []}));
    }
}
