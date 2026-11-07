//! Tiny browser entrypoint built by the production web pipeline.
#[cfg(feature = "web")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn greeting() -> String {
    "reproduced website".to_owned()
}
