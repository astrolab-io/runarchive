use crate::archive::Archive;
use wasm_bindgen::prelude::*;

/// The main entrypoint for Javascript/WASM consumers
#[wasm_bindgen(js_name = Archive)]
pub struct WasmArchive {
    inner: Archive,
}

#[wasm_bindgen]
impl WasmArchive {
    #[wasm_bindgen]
    pub async fn open(url: String) -> Result<WasmArchive, JsValue> {
        let inner = Archive::open(&url)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(WasmArchive { inner })
    }

    /// Returns a comma-separated string of filenames, or JSON.
    #[wasm_bindgen(js_name = listFiles)]
    pub fn list_files(&self) -> js_sys::Array {
        let iter = self
            .inner
            .list_files()
            .iter()
            .map(|e| JsValue::from_str(&e.name));
        let arr = js_sys::Array::new();
        for val in iter {
            arr.push(&val);
        }
        arr
    }

    /// Extracts the file yielding a Uint8Array byte slice
    #[wasm_bindgen(js_name = extractFile)]
    pub async fn extract_file(&mut self, filename: String) -> Result<js_sys::Uint8Array, JsValue> {
        let mut buffer = Vec::new();

        // Convert JS memory copy through the runtime
        self.inner
            .extract_file(&filename, &mut buffer)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        Ok(js_sys::Uint8Array::from(buffer.as_slice()))
    }
}
