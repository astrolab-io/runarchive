use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResumableCursor {
    pub compressed_offset: u64,
    pub uncompressed_offset: u64,
    pub bit_count: u8,
    pub bits: u8,
    pub dictionary_window: Vec<u8>,
}

pub fn get_cache_path(uri: &str, filename: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(uri.as_bytes());
    hasher.update(b"|");
    hasher.update(filename.as_bytes());
    let hash = hex::encode(hasher.finalize());
    let truncated = &hash[..10];
    
    let mut path = PathBuf::from(".runarchive");
    path.push(truncated);
    path
}

#[cfg(not(target_arch = "wasm32"))]
pub mod platform {
    use super::*;
    use std::fs;

    pub async fn load_checkpoint(uri: &str, filename: &str) -> Option<ResumableCursor> {
        let path = get_cache_path(uri, filename);
        if let Ok(data) = fs::read(&path) {
            bincode::deserialize(&data).ok()
        } else {
            None
        }
    }

    /// Saves a checkpoint atomically (async version for normal interval saves).
    pub async fn save_checkpoint(uri: &str, filename: &str, cursor: &ResumableCursor) -> std::io::Result<()> {
        save_checkpoint_sync(uri, filename, cursor)
    }

    /// Saves a checkpoint atomically using synchronous I/O.
    /// Used by `Drop` impls and signal handlers where async is unavailable.
    pub fn save_checkpoint_sync(uri: &str, filename: &str, cursor: &ResumableCursor) -> std::io::Result<()> {
        let path = get_cache_path(uri, filename);
        let tmp_path = path.with_extension("tmp");

        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let data = bincode::serialize(cursor)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        fs::write(&tmp_path, data)?;
        fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    pub async fn delete_checkpoint(uri: &str, filename: &str) {
        delete_checkpoint_sync(uri, filename);
    }

    /// Deletes a checkpoint synchronously.
    pub fn delete_checkpoint_sync(uri: &str, filename: &str) {
        let path = get_cache_path(uri, filename);
        let _ = fs::remove_file(&path);

        if let Some(parent) = path.parent() {
            if let Ok(mut entries) = fs::read_dir(parent) {
                if entries.next().is_none() {
                    let _ = fs::remove_dir(parent);
                }
            }
        }
    }
}

/// A drop guard that saves the last known checkpoint synchronously when dropped.
///
/// During extraction the loop continuously updates `latest_cursor` on every
/// DEFLATE block boundary. If the process is interrupted (Ctrl+C, panic, etc.)
/// the guard's `Drop` impl writes whatever the latest complete-block offset was
/// to the checkpoint file, ensuring the resumed append has no overlapping bytes.
///
/// Call `disarm()` on successful completion so the guard deletes the file instead.
#[cfg(not(target_arch = "wasm32"))]
pub struct CheckpointGuard {
    uri: String,
    filename: String,
    /// Holds the latest cursor to write on exit. Set to None to disarm (clean finish).
    latest_cursor: std::sync::Arc<std::sync::Mutex<Option<ResumableCursor>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl CheckpointGuard {
    /// Creates a new guard in the *armed* state (will save on drop).
    pub fn new(
        uri: &str,
        filename: &str,
        shared: std::sync::Arc<std::sync::Mutex<Option<ResumableCursor>>>,
    ) -> Self {
        Self {
            uri: uri.to_string(),
            filename: filename.to_string(),
            latest_cursor: shared,
        }
    }

    /// Disarms the guard. On drop it will delete the checkpoint instead of saving it.
    pub fn disarm(&self) {
        if let Ok(mut lock) = self.latest_cursor.lock() {
            *lock = None;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for CheckpointGuard {
    fn drop(&mut self) {
        let cursor_opt = self.latest_cursor.lock()
            .ok()
            .and_then(|g| g.clone());

        match cursor_opt {
            Some(cursor) => {
                // Still armed — save whatever the last block-boundary cursor was.
                if let Err(e) = platform::save_checkpoint_sync(&self.uri, &self.filename, &cursor) {
                    eprintln!("Warning: failed to write resume checkpoint on exit: {}", e);
                }
            }
            None => {
                // Disarmed by clean completion — remove the checkpoint file.
                platform::delete_checkpoint_sync(&self.uri, &self.filename);
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub mod platform {
    use super::*;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    async fn get_opfs_root() -> Result<JsValue, String> {
        let window = web_sys::window().ok_or("No window")?;
        let navigator = window.navigator();
        let storage = navigator.storage();
        let promise = storage.get_directory();
        JsFuture::from(promise).await.map_err(|e| format!("{:?}", e))
    }

    fn get_file_handle(root: &JsValue, name: &str, create: bool) -> js_sys::Promise {
        let options = js_sys::Object::new();
        js_sys::Reflect::set(&options, &"create".into(), &wasm_bindgen::JsValue::from_bool(create)).unwrap();
        let method = js_sys::Reflect::get(root, &"getFileHandle".into()).unwrap();
        let method = method.dyn_into::<js_sys::Function>().unwrap();
        method.call2(root, &wasm_bindgen::JsValue::from_str(name), &options).unwrap().into()
    }

    pub async fn load_checkpoint(uri: &str, filename: &str) -> Option<ResumableCursor> {
        let path = get_cache_path(uri, filename);
        let hash_name = path.file_name()?.to_str()?;
        
        let root = get_opfs_root().await.ok()?;
        
        let promise_handle = get_file_handle(&root, hash_name, false);
        let file_handle: JsValue = match JsFuture::from(promise_handle).await {
            Ok(v) => v,
            Err(_) => return None,
        };
        
        let method_get_file = js_sys::Reflect::get(&file_handle, &"getFile".into()).unwrap();
        let method_get_file = method_get_file.dyn_into::<js_sys::Function>().unwrap();
        let promise_file: js_sys::Promise = method_get_file.call0(&file_handle).unwrap().into();
        let file: JsValue = JsFuture::from(promise_file).await.ok()?;
        
        let method_array_buffer = js_sys::Reflect::get(&file, &"arrayBuffer".into()).unwrap();
        let method_array_buffer = method_array_buffer.dyn_into::<js_sys::Function>().unwrap();
        let promise_buf: js_sys::Promise = method_array_buffer.call0(&file).unwrap().into();
        let array_buffer: JsValue = JsFuture::from(promise_buf).await.ok()?;
        
        let uint8_array = js_sys::Uint8Array::new(&array_buffer);
        let mut data = vec![0u8; uint8_array.length() as usize];
        uint8_array.copy_to(&mut data);
        
        bincode::deserialize(&data).ok()
    }

    pub async fn save_checkpoint(uri: &str, filename: &str, cursor: &ResumableCursor) -> std::io::Result<()> {
        let path = get_cache_path(uri, filename);
        let hash_name = path.file_name().unwrap().to_str().unwrap();
        
        let root = get_opfs_root().await.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        
        let promise_handle = get_file_handle(&root, hash_name, true);
        let file_handle: JsValue = JsFuture::from(promise_handle).await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", e)))?;
        
        let data = bincode::serialize(cursor)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let uint8_array = js_sys::Uint8Array::from(data.as_slice());
        
        let method_create_writable = js_sys::Reflect::get(&file_handle, &"createWritable".into()).unwrap();
        let method_create_writable = method_create_writable.dyn_into::<js_sys::Function>().unwrap();
        let promise_writable: js_sys::Promise = method_create_writable.call0(&file_handle).unwrap().into();
        let stream: JsValue = JsFuture::from(promise_writable).await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", e)))?;
            
        let method_write = js_sys::Reflect::get(&stream, &"write".into()).unwrap();
        let method_write = method_write.dyn_into::<js_sys::Function>().unwrap();
        let promise_write: js_sys::Promise = method_write.call1(&stream, &uint8_array).unwrap().into();
        JsFuture::from(promise_write).await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", e)))?;
            
        let method_close = js_sys::Reflect::get(&stream, &"close".into()).unwrap();
        let method_close = method_close.dyn_into::<js_sys::Function>().unwrap();
        let promise_close: js_sys::Promise = method_close.call0(&stream).unwrap().into();
        JsFuture::from(promise_close).await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", e)))?;
        
        Ok(())
    }

    pub async fn delete_checkpoint(uri: &str, filename: &str) {
        let path = get_cache_path(uri, filename);
        if let Some(hash_name) = path.file_name().and_then(|n| n.to_str()) {
            if let Ok(root) = get_opfs_root().await {
                let method_remove = js_sys::Reflect::get(&root, &"removeEntry".into()).unwrap();
                let method_remove = method_remove.dyn_into::<js_sys::Function>().unwrap();
                let promise_remove: js_sys::Promise = method_remove.call1(&root, &wasm_bindgen::JsValue::from_str(hash_name)).unwrap().into();
                let _ = JsFuture::from(promise_remove).await;
            }
        }
    }
}

pub fn calculate_checkpoint_interval(uncompressed_size: u64) -> u64 {
    let one_percent = uncompressed_size / 100;
    let min_interval = 5 * 1024 * 1024; // 5 MB
    let max_interval = 25 * 1024 * 1024; // 25 MB
    
    one_percent.clamp(min_interval, max_interval)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_checkpoint_interval() {
        // Small file, should be clamped to 5 MB
        assert_eq!(calculate_checkpoint_interval(1024), 5 * 1024 * 1024);
        
        // Mid file (1GB), 1% is ~10MB, which is within bounds (5MB - 25MB)
        assert_eq!(calculate_checkpoint_interval(1024 * 1024 * 1024), 10737418); 
        // 1024*1024*1024 / 100 = 10737418
        
        // Huge file (10GB), 1% is ~100MB, should be clamped to 25 MB
        assert_eq!(calculate_checkpoint_interval(10u64 * 1024 * 1024 * 1024), 25 * 1024 * 1024);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn test_platform_checkpointing() {
        let uri = "http://example.com/test.zip";
        let filename = "large_file.bin";
        
        // Ensure cleanup before test
        platform::delete_checkpoint(uri, filename).await;
        
        let cursor = ResumableCursor {
            compressed_offset: 12345,
            uncompressed_offset: 67890,
            bit_count: 5,
            bits: 12,
            dictionary_window: vec![1, 2, 3, 4, 5],
        };
        
        // Load should be None
        assert!(platform::load_checkpoint(uri, filename).await.is_none());
        
        // Save checkpoint
        platform::save_checkpoint(uri, filename, &cursor).await.expect("Failed to save checkpoint");
        
        // Load checkpoint
        let loaded = platform::load_checkpoint(uri, filename).await.expect("Failed to load checkpoint");
        assert_eq!(loaded.compressed_offset, cursor.compressed_offset);
        assert_eq!(loaded.uncompressed_offset, cursor.uncompressed_offset);
        assert_eq!(loaded.dictionary_window, cursor.dictionary_window);
        
        // Delete checkpoint
        platform::delete_checkpoint(uri, filename).await;
        assert!(platform::load_checkpoint(uri, filename).await.is_none());
    }
}
