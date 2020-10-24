use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    // https://rustwasm.github.io/docs/wasm-bindgen/print.html
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

macro_rules! console_log {
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}

#[wasm_bindgen(start)]
pub fn main() {
    console_log!("Hello, world!");
}
